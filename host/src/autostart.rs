//! Autostart: the operating system's service manager starts a Host by itself, at login and again
//! after a failure (ADR 0014). One entry per Host: a launchd agent on macOS, a systemd user unit on
//! Linux. tooob never keeps a Host running itself.
use crate::hosts::{Entry, quote};
use anyhow::{Context, Result, anyhow, bail};
use std::path::{Path, PathBuf};

/// The service manager on this box, found by its tool on `PATH`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ServiceManager {
    Launchd,
    Systemd,
}

impl ServiceManager {
    /// launchd where there's `launchctl` (macOS), systemd where there's `systemctl` (Linux).
    pub fn here() -> Option<ServiceManager> {
        if on_path("launchctl") {
            Some(ServiceManager::Launchd)
        } else if on_path("systemctl") {
            Some(ServiceManager::Systemd)
        } else {
            None
        }
    }
}

/// Whether an executable named `tool` is on `PATH`.
fn on_path(tool: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| executable(&dir.join(tool)))
}

fn executable(file: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(file).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Where the install script puts tooob: a path that stays the same across upgrades, so an entry
/// runs the new Release once the Host restarts.
pub fn installed_tooob() -> Result<PathBuf> {
    Ok(home_dir()?.join(".local/bin/tooob"))
}

fn home_dir() -> Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var_os("HOME").context("HOME isn't set")?,
    ))
}

/// Whether Autostart is on for `host`: its entry is there.
pub fn is_on(host: &Entry) -> bool {
    entry_file(host).is_some_and(|file| file.exists())
}

/// Where `host`'s entry is, or would be, for the service manager here; None without one.
pub fn entry_file(host: &Entry) -> Option<PathBuf> {
    let manager = ServiceManager::here()?;
    let home = home_dir().ok()?;
    Some(match manager {
        ServiceManager::Launchd => home
            .join("Library/LaunchAgents")
            .join(format!("{}.plist", label(&host.name))),
        ServiceManager::Systemd => {
            let config = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .filter(|dir| dir.is_absolute())
                .unwrap_or_else(|| home.join(".config"));
            config.join("systemd/user").join(unit(&host.name))
        }
    })
}

/// launchd's name for a Host's agent.
fn label(name: &str) -> String {
    format!("net.tooobs.{name}")
}

/// systemd's name for a Host's unit: a unit file of its own for each Host, named like an instance
/// of a template (`tooob@.service`) though there's no template.
fn unit(name: &str) -> String {
    format!("tooob@{name}.service")
}

/// launchd's domain for this user's agents.
fn launchd_domain() -> String {
    // SAFETY: getuid can't fail, and touches no memory of ours.
    format!("gui/{}", unsafe { libc::getuid() })
}

/// launchd's name for a Host's agent, loaded in this user's domain.
fn launchd_service(name: &str) -> String {
    format!("{}/{}", launchd_domain(), label(name))
}

/// Turn Autostart on for `host`, and start it now: through its service manager, from scratch if
/// it was on already. Nothing is left behind when the service manager refuses. What to tell the
/// Owner, a line each.
pub fn enable(host: &Entry) -> Result<Vec<String>> {
    let neither = || {
        anyhow!(
            "enable needs systemd or launchd, and this system has neither.\n  Have your service \
             manager run `~/.local/bin/tooob run {}` as you, restarting it on failure.",
            host.name
        )
    };
    let manager = ServiceManager::here().ok_or_else(neither)?;
    if manager == ServiceManager::Systemd && !systemd_booted() {
        return Err(neither());
    }
    let tooob = installed_tooob()?;
    if !tooob.exists() {
        let this = std::env::current_exe()?;
        bail!(
            "{} would start from {}, where the install script puts tooob, and it isn't there: \
             install tooob, or put this one there:\n  mkdir -p {} && cp {} {}",
            host.name,
            tooob.display(),
            quote(path(tooob.parent().unwrap())?),
            quote(path(&this)?),
            quote(path(&tooob)?),
        );
    }
    let file = entry_file(host).context("no entry")?;
    let was_on = file.exists();
    if let Some(pid) = host.running().filter(|_| !was_on) {
        bail!(
            "{0} is running here (pid {pid}), started by hand: stop it first (Ctrl-C where it \
             runs, or `tooob stop {0}`), then enable it",
            host.name
        );
    }
    if was_on && manager == ServiceManager::Launchd {
        // launchd loads an agent once: unload it (stopping the Host) to load what's written now.
        let _ = call("launchctl", &["bootout", &launchd_service(&host.name)]);
        host.until_stopped()?;
    }
    std::fs::create_dir_all(file.parent().unwrap())?;
    let text = match manager {
        ServiceManager::Launchd => plist(host, &tooob)?,
        ServiceManager::Systemd => systemd_unit(host, &tooob)?,
    };
    std::fs::write(&file, text)?;
    let started = match manager {
        ServiceManager::Launchd => {
            let load = || call("launchctl", &["bootstrap", &launchd_domain(), path(&file)?]);
            // Just unloaded, launchd may still be letting go of it for a moment.
            let mut loaded = load();
            for _ in 0..5 {
                if loaded.is_ok() || !was_on {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
                loaded = load();
            }
            loaded
        }
        ServiceManager::Systemd => {
            let unit = unit(&host.name);
            systemctl(&["daemon-reload"])
                .and_then(|()| systemctl(&["enable", &unit]))
                .and_then(|()| systemctl(&["restart", &unit]))
                .inspect_err(|_| {
                    let _ = systemctl(&["disable", &unit]);
                })
        }
    };
    if let Err(e) = started {
        let _ = std::fs::remove_file(&file);
        if manager == ServiceManager::Systemd {
            let _ = systemctl(&["daemon-reload"]);
        }
        return Err(e);
    }
    let name = &host.name;
    let mut said = vec![format!(
        "{name} starts by itself now, at login and again after a failure; `tooob logs {name} -f` \
         follows its log."
    )];
    said.push(match manager {
        ServiceManager::Launchd => {
            "While it serves, this Mac won't idle-sleep, but closing its lid still sleeps it."
                .into()
        }
        ServiceManager::Systemd => "On a box nobody stays logged in to, `loginctl enable-linger` \
                                    keeps it running from boot, logged in or not."
            .into(),
    });
    Ok(said)
}

/// Turn Autostart off for `host`: stop it through its service manager and remove its entry, which
/// goes whatever the service manager says. Nothing to do when it's off.
pub fn disable(host: &Entry) -> Result<()> {
    let Some(file) = entry_file(host).filter(|file| file.exists()) else {
        return Ok(());
    };
    let stopped = match ServiceManager::here() {
        Some(ServiceManager::Launchd) => {
            call("launchctl", &["bootout", &launchd_service(&host.name)])
        }
        Some(ServiceManager::Systemd) => systemctl(&["disable", "--now", &unit(&host.name)]),
        None => Ok(()),
    };
    std::fs::remove_file(&file)?;
    if ServiceManager::here() == Some(ServiceManager::Systemd) {
        let _ = systemctl(&["daemon-reload"]);
    }
    // launchd refuses to unload an agent it never loaded (one enabled outside a login session):
    // that's only a problem if the Host still runs.
    match host.until_stopped() {
        Ok(()) => Ok(()),
        Err(e) => Err(stopped.err().unwrap_or(e)),
    }
}

/// Stop `host`, whose Autostart is on, through its service manager: it stays stopped, and starts
/// by itself again at the next login.
pub fn stop(host: &Entry) -> Result<()> {
    match ServiceManager::here() {
        Some(ServiceManager::Launchd) => call(
            "launchctl",
            &["kill", "SIGTERM", &launchd_service(&host.name)],
        )?,
        Some(ServiceManager::Systemd) => systemctl(&["stop", &unit(&host.name)])?,
        None => bail!("no service manager runs {}", host.name),
    }
    host.until_stopped()
}

/// Restart `host`, whose Autostart is on and which runs as `pid`, through its service manager, so
/// it runs the Release now installed; and wait, up to 10 s, until it's running again. launchd is
/// asked to stop it cleanly (it isn't started again after a clean exit) and then to start it.
pub fn restart(host: &Entry, pid: u32) -> Result<()> {
    let service = launchd_service(&host.name);
    match ServiceManager::here() {
        Some(ServiceManager::Launchd) => {
            call("launchctl", &["kill", "SIGTERM", &service])?;
            host.until_stopped()?;
            call("launchctl", &["kickstart", &service])?;
        }
        Some(ServiceManager::Systemd) => systemctl(&["restart", &unit(&host.name)])?,
        None => bail!("no service manager runs {}", host.name),
    }
    let started = std::time::Instant::now();
    while host.running().is_none_or(|now| now == pid) {
        if started.elapsed() > std::time::Duration::from_secs(10) {
            bail!("{} didn't start again within 10 s", host.name);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Ok(())
}

/// Whether this process is `host`'s entry's own `tooob run`: its service manager's.
pub fn started_by_its_entry(host: &Entry) -> bool {
    std::env::var(AUTOSTART).is_ok_and(|name| name == host.name)
}

fn path(file: &Path) -> Result<&str> {
    file.to_str()
        .with_context(|| format!("{} isn't UTF-8", file.display()))
}

/// Run `tool` with `args`; its error output if it fails.
fn call(tool: &str, args: &[&str]) -> Result<()> {
    let out = std::process::Command::new(tool)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .with_context(|| format!("can't run {tool}"))?;
    if !out.status.success() {
        bail!(
            "`{tool} {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Whether systemd is what booted this box: `systemctl` is there in a container or on WSL, say,
/// where it isn't, and then says "offline".
fn systemd_booted() -> bool {
    std::process::Command::new("systemctl")
        .arg("is-system-running")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).trim() != "offline")
}

/// `systemctl --user ARGS`.
fn systemctl(args: &[&str]) -> Result<()> {
    call("systemctl", &[&["--user"], args].concat())
}

/// What the entry sets in the Host's environment: that it's the entry's own `tooob run` (which
/// `run` allows for an enabled Host), where its Hosts are, when that isn't the default, and that
/// its release check is off, when `TOOOB_NO_UPDATE_CHECK` says so as it's enabled.
fn environment(host: &Entry) -> Result<Vec<(&'static str, String)>> {
    let mut env = vec![(AUTOSTART, host.name.clone())];
    if crate::upgrade::release_check_off() {
        env.push(("TOOOB_NO_UPDATE_CHECK", "1".to_string()));
    }
    if let Some(dir) = std::env::var_os("TOOOB_HOME") {
        let dir = std::path::absolute(dir)?;
        env.push(("TOOOB_HOME", path(&dir)?.to_string()));
    } else {
        for var in ["XDG_CONFIG_HOME", "XDG_STATE_HOME"] {
            if let Some(dir) = std::env::var_os(var).map(PathBuf::from)
                && dir.is_absolute()
            {
                env.push((var, path(&dir)?.to_string()));
            }
        }
    }
    Ok(env)
}

/// Set in a Host's environment by its entry, to its Host Name: this `tooob run` is the service
/// manager's.
pub const AUTOSTART: &str = "TOOOB_AUTOSTART";

/// A launchd agent for `host`: run at load (login), kept alive while it exits with a failure, with
/// its output in its log. Interactive, so macOS doesn't throttle what it serves.
fn plist(host: &Entry, tooob: &Path) -> Result<String> {
    let string = |s: &str| {
        format!(
            "<string>{}</string>",
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        )
    };
    let log = string(path(&host.log_file())?);
    let env: String = environment(host)?
        .iter()
        .map(|(k, v)| format!("\t\t<key>{k}</key>\n\t\t{}\n", string(v)))
        .collect();
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!-- tooob's Autostart for the Host its Label names: `tooob disable` removes it. -->
<plist version="1.0">
<dict>
	<key>Label</key>
	{label}
	<key>ProgramArguments</key>
	<array>
		{tooob}
		<string>run</string>
		{name_string}
	</array>
	<key>EnvironmentVariables</key>
	<dict>
{env}	</dict>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<dict>
		<key>SuccessfulExit</key>
		<false/>
	</dict>
	<key>ProcessType</key>
	<string>Interactive</string>
	<key>StandardOutPath</key>
	{log}
	<key>StandardErrorPath</key>
	{log}
</dict>
</plist>
"#,
        label = string(&label(&host.name)),
        tooob = string(path(tooob)?),
        name_string = string(&host.name),
    ))
}

/// A systemd user unit for `host`: started with the user's own service manager (at login, or at
/// boot with lingering), restarted when it exits with a failure, with its output in its log.
fn systemd_unit(host: &Entry, tooob: &Path) -> Result<String> {
    // Quoted, with systemd's own specifiers (`%`) escaped.
    let quoted = |s: &str| {
        format!(
            "\"{}\"",
            s.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('%', "%%")
        )
    };
    let log = path(&host.log_file())?.replace('%', "%%");
    let env: String = environment(host)?
        .iter()
        .map(|(k, v)| format!("Environment={}\n", quoted(&format!("{k}={v}"))))
        .collect();
    Ok(format!(
        "# tooob's Autostart for the Host {name}: `tooob disable {name}` removes it.\n\
         [Unit]\n\
         Description=tooob Host {name}\n\
         \n\
         [Service]\n\
         ExecStart={tooob} run {name}\n\
         {env}\
         Restart=on-failure\n\
         RestartSec=5\n\
         StandardOutput=append:{log}\n\
         StandardError=append:{log}\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        name = host.name,
        tooob = quoted(path(tooob)?),
    ))
}

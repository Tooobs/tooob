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

/// What keeping a Host running here takes, checked: this box's service manager, and tooob where
/// the install script puts it, which the Host's entry runs.
pub struct Keeper {
    manager: ServiceManager,
    tooob: PathBuf,
}

/// Check that the Host `name` can be kept running here, before anything changes, or bail saying
/// what to type instead.
pub fn require_keeping_running(name: &str) -> Result<Keeper> {
    let neither = || {
        anyhow!(
            "keeping {0} running needs systemd or launchd, and this system has neither.\n  Have \
             your service manager run `~/.local/bin/tooob run {0}` as you, restarting it on \
             failure.",
            name
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
            name,
            tooob.display(),
            quote(path(tooob.parent().unwrap())?),
            quote(path(&this)?),
            quote(path(&tooob)?),
        );
    }
    Ok(Keeper { manager, tooob })
}

impl Keeper {
    /// The service manager that keeps it running: `launchd` or `systemd`.
    pub fn service_manager(&self) -> &'static str {
        match self.manager {
            ServiceManager::Launchd => "launchd",
            ServiceManager::Systemd => "systemd",
        }
    }

    /// `-d`: keep `host` running, now and after logins, reboots and crashes, by writing its entry
    /// and starting it through its service manager, from scratch if it had one already. tooob
    /// never backgrounds it itself (ADR 0014). Refused while it runs by hand. Nothing is left
    /// behind when the service manager refuses. What to tell the Owner, a line each.
    pub fn keep_running(&self, host: &Entry) -> Result<Vec<String>> {
        let manager = self.manager;
        let file = entry_file(host).context("no entry")?;
        let was_on = file.exists();
        if was_on && host.running().is_some() {
            return Ok(vec![already_running(&host.name)]);
        }
        if let Some(pid) = host.running() {
            bail!(
                "{0} is running here (pid {pid}), started by hand: stop it first (Ctrl-C where it \
                 runs, or `tooob stop {0}`), then `tooob run {0} -d`",
                host.name
            );
        }
        self.write(host, &file, crate::upgrade::release_check_off())?;
        if let Err(e) = self.load(host, &file, was_on) {
            let _ = self.withdraw(&file, None);
            return Err(e);
        }
        // Kept running again: whatever `tooob stop` put by is done with.
        forget_put_by(host);
        let name = &host.name;
        let mut said = vec![kept_running(name)];
        said.push(match manager {
            ServiceManager::Launchd => {
                "While it serves, this Mac won't idle-sleep, but closing its lid still sleeps it."
                    .into()
            }
            ServiceManager::Systemd => "On a box nobody stays logged in to, `loginctl \
                                        enable-linger` keeps it running from boot, logged in or \
                                        not."
                .into(),
        });
        Ok(said)
    }

    /// Hand `host`'s entry, `file`, to its service manager, which starts it now: from scratch
    /// when `was_on`, since one may be loaded already (launchd loads an agent once).
    fn load(&self, host: &Entry, file: &Path, was_on: bool) -> Result<()> {
        crate::log::make_crash_file(&host.state_dir)?;
        match self.manager {
            ServiceManager::Launchd => {
                if was_on {
                    // Unloading it stops the Host, so it's waited for.
                    let _ = call("launchctl", &["bootout", &launchd_service(&host.name)]);
                    host.until_stopped()?;
                }
                let load = || call("launchctl", &["bootstrap", &launchd_domain(), path(file)?]);
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
        }
    }

    /// Write `host`'s entry, `file`, as this tooob writes it, for the Host as it's found now.
    fn write(&self, host: &Entry, file: &Path, release_check_off: bool) -> Result<()> {
        std::fs::create_dir_all(file.parent().unwrap())?;
        let text = match self.manager {
            ServiceManager::Launchd => plist(host, &self.tooob, release_check_off)?,
            ServiceManager::Systemd => systemd_unit(host, &self.tooob, release_check_off)?,
        };
        Ok(std::fs::write(file, text)?)
    }

    /// Take `file`, an entry its service manager wouldn't load, out of its sight again: removed,
    /// or moved to `to`.
    fn withdraw(&self, file: &Path, to: Option<&Path>) -> Result<()> {
        let gone = match to {
            Some(to) => move_file(file, to),
            None => Ok(std::fs::remove_file(file)?),
        };
        if self.manager == ServiceManager::Systemd {
            let _ = systemctl(&["daemon-reload"]);
        }
        gone
    }
}

/// `tooob start`: start `host`, which isn't running, the way it ran before: through its service
/// manager, which keeps it running again. Its entry is written afresh, for where the Host is now
/// (its Hosts' folder may have moved since `-d`), keeping only what `-d` was typed with that the
/// Host can't say: whether its release check is off. A Host `tooob stop` keeps stopped, or one
/// kept running that isn't running now (between crashes, say), starts; one only ever run in a
/// terminal says what to type (ADR 0022). If its service manager refuses, it's kept stopped. What
/// to tell the Owner, a line each.
pub fn start(host: &Entry) -> Result<Vec<String>> {
    let name = &host.name;
    let (Some(file), Some(stopping), Some(stopped)) = (
        entry_file(host),
        put_by(host, "stopping"),
        put_by(host, "stopped"),
    ) else {
        bail!("{}", doesnt_start_by_itself(name));
    };
    // Its entry, or what `stop` put by, newest first.
    let Some(was) = [&file, &stopping, &stopped]
        .into_iter()
        .find(|f| f.exists())
    else {
        bail!("{}", doesnt_start_by_itself(name));
    };
    let keeper = require_keeping_running(name)?;
    let release_check_off = crate::upgrade::release_check_off()
        || std::fs::read_to_string(was).is_ok_and(|text| text.contains("TOOOB_NO_UPDATE_CHECK"));
    // Only a stop that finished unloaded it: kept running, or a stop cut short, it may still be
    // loaded (launchd loads an agent once).
    let loaded = was != &stopped;
    keeper.write(host, &file, release_check_off)?;
    if let Err(e) = keeper.load(host, &file, loaded) {
        // Kept stopped rather than kept running: an entry its service manager won't load would
        // only fail again at the next login.
        forget_put_by(host);
        return Err(match keeper.withdraw(&file, Some(&stopped)) {
            Ok(()) => e.context(format!(
                "{name} is still stopped, and kept stopped: `tooob start {name}` tries again"
            )),
            Err(b) => e.context(format!(
                "{name} didn't start, and its entry couldn't be put by ({b:#}): `tooob stop \
                 {name}` keeps it stopped"
            )),
        });
    }
    forget_put_by(host);
    Ok(vec![kept_running(name)])
}

/// What `start` says of a Host only ever run in a terminal.
fn doesnt_start_by_itself(name: &str) -> String {
    format!(
        "{name} doesn't start by itself; serve it here with `tooob run {name}`, or keep it \
         running with `tooob run {name} -d`"
    )
}

/// What `keep_running` says first: that `name` is kept running, and how to follow and stop it.
fn kept_running(name: &str) -> String {
    format!(
        "{name} is kept running now, and after logins, reboots and crashes: `tooob logs {name} \
         -f` follows it, and `tooob stop {name}` stops it."
    )
}

/// What's said of a Host its service manager is running already.
pub fn already_running(name: &str) -> String {
    format!(
        "{name} is already running in the background: `tooob logs {name} -f` follows it, and \
         `tooob stop {name}` stops it"
    )
}

/// For `tooob rm`: stop `host` through its service manager and remove its entry, which goes
/// whatever the service manager says, and whatever `tooob stop` put by. Nothing to do when it was
/// never kept running.
pub fn remove(host: &Entry) -> Result<()> {
    forget_put_by(host);
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
    // launchd refuses to unload an agent it never loaded (one kept running from outside a login
    // session): that's only a problem if the Host still runs.
    match host.until_stopped() {
        Ok(()) => Ok(()),
        Err(e) => Err(stopped.err().unwrap_or(e)),
    }
}

/// `tooob stop` on `host`: if it's kept running, or a stop of it was cut short, stop it through
/// its service manager and keep it stopped, across logins and reboots; false when its service
/// manager has nothing to do with it.
///
/// Its entry goes out of the service manager's sight first, into its state folder (systemd's
/// unit is disabled just before, which needs the file), so whatever happens next, no login or
/// reboot brings the Host back; then it's stopped. If it won't stop, its entry is put back, and
/// it's still kept running. The entry is put by as `.stopping` until the Host has stopped, then
/// `.stopped`, where `start` finds it: the next `stop` finishes one cut short in between.
pub fn stop(host: &Entry) -> Result<bool> {
    let (Some(entry), Some(stopping), Some(stopped)) = (
        entry_file(host),
        put_by(host, "stopping"),
        put_by(host, "stopped"),
    ) else {
        return Ok(false);
    };
    let name = &host.name;
    let fresh = entry.exists();
    if !fresh && !stopping.exists() {
        return Ok(false);
    }
    let systemd = ServiceManager::here() == Some(ServiceManager::Systemd);
    let unit = unit(name);
    let mut put = Ok(());
    if fresh {
        if systemd {
            let _ = systemctl(&["disable", &unit]);
        }
        put = move_file(&entry, &stopping);
        if let Err(e) = &put
            && !systemd
        {
            bail!("{name} wasn't stopped: its entry couldn't be put by ({e:#})");
        }
    }
    let asked = match ServiceManager::here() {
        Some(ServiceManager::Launchd) => call("launchctl", &["bootout", &launchd_service(name)]),
        Some(ServiceManager::Systemd) => systemctl(&["stop", &unit]),
        None => Ok(()),
    };
    if systemd {
        let _ = systemctl(&["daemon-reload"]);
    }
    // launchd refuses to unload an agent it hasn't loaded (between crashes, or one kept running
    // from outside a login session): that's only a problem if the Host still runs.
    if let Err(e) = host.until_stopped() {
        let why = asked.err().unwrap_or(e);
        if !fresh {
            return Err(why);
        }
        let back = match &put {
            Ok(()) => move_file(&stopping, &entry),
            Err(_) => Ok(()),
        };
        if systemd {
            let _ = systemctl(&["daemon-reload"]);
            let _ = systemctl(&["enable", &unit]);
        }
        return Err(match back {
            Ok(()) => anyhow!("{name} didn't stop, so it's still kept running: {why:#}"),
            Err(e) => anyhow!(
                "{name} didn't stop ({why:#}), and its entry couldn't be put back ({e:#}): \
                 move {} to {} to keep it running",
                stopping.display(),
                entry.display()
            ),
        });
    }
    let done = put.and_then(|()| Ok(std::fs::rename(&stopping, &stopped)?));
    if let Err(e) = done {
        let left = if entry.exists() { &entry } else { &stopping };
        bail!(
            "{name} is stopped, but its entry couldn't be put by ({e:#}): it's still at {}, and \
             `tooob stop {name}` again finishes the job",
            left.display()
        );
    }
    Ok(true)
}

/// Whether `host` was kept running and `tooob stop` stopped it: its entry is put by, for `tooob
/// start` to start it again the way it ran.
pub fn is_kept_stopped(host: &Entry) -> bool {
    ["stopped", "stopping"]
        .iter()
        .any(|state| put_by(host, state).is_some_and(|file| file.exists()))
}

/// Where `tooob stop` puts `host`'s entry by, as `state` (`stopping`, then `stopped`): in its
/// state folder, out of its service manager's sight. None without a service manager.
fn put_by(host: &Entry, state: &str) -> Option<PathBuf> {
    let entry = entry_file(host)?;
    let name = entry.file_name()?.to_string_lossy().into_owned();
    Some(host.state_dir.join(format!("{name}.{state}")))
}

/// Forget whatever `tooob stop` put by for `host`. Deliberately quiet: a file that isn't there,
/// or can't go, changes nothing about whether the Host is kept running, which only its entry says.
fn forget_put_by(host: &Entry) {
    for state in ["stopped", "stopping"] {
        if let Some(file) = put_by(host, state) {
            let _ = std::fs::remove_file(file);
        }
    }
}

/// Move `from` to `to`, which may be on another file system (a `TOOOB_HOME` elsewhere). On
/// failure, `from` is as it was and nothing is left at `to`.
fn move_file(from: &Path, to: &Path) -> Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    let copied = std::fs::copy(from, to).and_then(|_| std::fs::remove_file(from));
    if let Err(e) = copied {
        let _ = std::fs::remove_file(to);
        return Err(e.into());
    }
    Ok(())
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
/// `run` allows for a Host kept running), where its Hosts are, when that isn't the default, and
/// that its release check is off, when `release_check_off`.
fn environment(host: &Entry, release_check_off: bool) -> Result<Vec<(&'static str, String)>> {
    let mut env = vec![(AUTOSTART, host.name.clone())];
    if release_check_off {
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
/// its output, only its dying words since it writes its own log, in its crash log. Interactive, so
/// macOS doesn't throttle what it serves.
fn plist(host: &Entry, tooob: &Path, release_check_off: bool) -> Result<String> {
    let string = |s: &str| {
        format!(
            "<string>{}</string>",
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        )
    };
    let crashes = string(path(&crate::log::crash_file(&host.state_dir))?);
    let env: String = environment(host, release_check_off)?
        .iter()
        .map(|(k, v)| format!("\t\t<key>{k}</key>\n\t\t{}\n", string(v)))
        .collect();
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!-- tooob's Autostart for the Host its Label names: `tooob stop` stops it, and `tooob rm` removes it. -->
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
	{crashes}
	<key>StandardErrorPath</key>
	{crashes}
</dict>
</plist>
"#,
        label = string(&label(&host.name)),
        tooob = string(path(tooob)?),
        name_string = string(&host.name),
    ))
}

/// A systemd user unit for `host`: started with the user's own service manager (at login, or at
/// boot with lingering), restarted when it exits with a failure, with its output, only its dying
/// words since it writes its own log, in its crash log.
fn systemd_unit(host: &Entry, tooob: &Path, release_check_off: bool) -> Result<String> {
    // Quoted, with systemd's own specifiers (`%`) escaped.
    let quoted = |s: &str| {
        format!(
            "\"{}\"",
            s.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('%', "%%")
        )
    };
    let crashes = path(&crate::log::crash_file(&host.state_dir))?.replace('%', "%%");
    let env: String = environment(host, release_check_off)?
        .iter()
        .map(|(k, v)| format!("Environment={}\n", quoted(&format!("{k}={v}"))))
        .collect();
    Ok(format!(
        "# tooob's Autostart for the Host {name}: `tooob stop {name}` stops it, and `tooob rm \
         {name}` removes it.\n\
         [Unit]\n\
         Description=tooob Host {name}\n\
         \n\
         [Service]\n\
         ExecStart={tooob} run {name}\n\
         {env}\
         Restart=on-failure\n\
         RestartSec=5\n\
         StandardOutput=append:{crashes}\n\
         StandardError=append:{crashes}\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        name = host.name,
        tooob = quoted(path(tooob)?),
    ))
}

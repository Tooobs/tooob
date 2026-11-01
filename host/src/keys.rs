//! The keys a Host run in a terminal reads: `q` quits and `d` detaches (#97). Only while it's the
//! terminal's foreground job: started with `&`, or with no terminal, it reads nothing, and a job
//! the shell puts in the background never reads, so the shell never stops it for reading (ADR
//! 0022: reading a key isn't asking a question).
//!
//! While it reads, the terminal takes keys one at a time and doesn't echo them; everything else
//! is as it was, so Ctrl-C and Ctrl-Z still signal, and lines printed meanwhile still start at the
//! left. The terminal is put back as it was however the Host ends (a key, a signal, an error, a
//! panic), and whenever it's suspended (Ctrl-Z): `fg` brings the keys back.
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI32, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;

/// A key the Host acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// `q`: stop, as Ctrl-C does.
    Quit,
    /// `d`: hand the Host to its service manager and give the terminal back.
    Detach,
}

/// Whether this process is its terminal's foreground job: its standard input is a terminal, and
/// its process group is the one the terminal reads for.
pub fn in_the_foreground() -> bool {
    // SAFETY: isatty, tcgetpgrp and getpgrp only read the process's and the terminal's state.
    unsafe { libc::isatty(STDIN) == 1 && libc::tcgetpgrp(STDIN) == libc::getpgrp() }
}

const STDIN: libc::c_int = libc::STDIN_FILENO;

/// Reads the Host's keys from its terminal, while it's the foreground job, until dropped.
pub struct Keys {
    keys: mpsc::UnboundedReceiver<Key>,
    done: Arc<AtomicBool>,
    reader: Option<std::thread::JoinHandle<()>>,
    /// What SIGTSTP did before, put back when done.
    was: Option<libc::sigaction>,
}

impl Keys {
    /// Start reading keys; None with no terminal on standard input.
    pub fn read() -> Option<Keys> {
        // SAFETY: isatty only asks.
        if unsafe { libc::isatty(STDIN) } != 1 {
            return None;
        }
        install_panic_hook();
        let (send, keys) = mpsc::unbounded_channel();
        let done = Arc::new(AtomicBool::new(false));
        let reading = done.clone();
        let reader = std::thread::Builder::new()
            .name("keys".into())
            .spawn(move || read_keys(&send, &reading))
            .ok()?;
        // Once there's a reader to suspend the process: until then, Ctrl-Z does as it always did.
        let was = on_suspend();
        Some(Keys {
            keys,
            done,
            reader: Some(reader),
            was,
        })
    }

    /// The next key pressed.
    pub async fn next(&mut self) -> Key {
        match self.keys.recv().await {
            Some(key) => key,
            None => std::future::pending().await,
        }
    }
}

impl Drop for Keys {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        put_back();
        if let Some(was) = &self.was {
            // SAFETY: `was` is what sigaction gave back for SIGTSTP.
            unsafe { libc::sigaction(libc::SIGTSTP, was, std::ptr::null_mut()) };
        }
    }
}

/// How long the reader waits for a key before it looks again at whether it's still to read.
const LOOK_AGAIN: Duration = Duration::from_millis(100);

/// Read keys until `done`, while in the foreground; the terminal as it was once it's done. It's
/// the reader alone that changes the terminal, so a Ctrl-Z comes to it too (`suspended`), to put
/// the terminal back before the process stops.
fn read_keys(send: &mpsc::UnboundedSender<Key>, done: &AtomicBool) {
    while !done.load(Ordering::SeqCst) {
        let reading = in_the_foreground() && take_keys_singly();
        let mut ready = [
            libc::pollfd {
                fd: SUSPEND.read.load(Ordering::SeqCst),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: STDIN,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let watched = if reading { 2 } else { 1 };
        // SAFETY: `watched` pollfds, ours.
        let polled = unsafe {
            libc::poll(
                ready.as_mut_ptr(),
                watched,
                LOOK_AGAIN.as_millis() as libc::c_int,
            )
        };
        if polled <= 0 {
            continue;
        }
        if ready[0].revents & libc::POLLIN != 0 {
            suspend();
            continue;
        }
        if ready[1].revents & libc::POLLIN == 0 {
            std::thread::sleep(LOOK_AGAIN); // hung up: nothing more to read
            continue;
        }
        // Read only in the foreground: a background job that reads is stopped by its shell.
        if !in_the_foreground() {
            continue;
        }
        let mut byte = 0u8;
        // SAFETY: one byte, into ours.
        let read = unsafe { libc::read(STDIN, (&raw mut byte).cast(), 1) };
        if read != 1 {
            std::thread::sleep(LOOK_AGAIN);
            continue;
        }
        let key = match byte {
            b'q' | b'Q' => Key::Quit,
            b'd' | b'D' => Key::Detach,
            _ => continue,
        };
        if send.send(key).is_err() {
            break;
        }
    }
    put_back();
}

/// The terminal as it was before keys were taken singly, while they are.
struct Saved(std::cell::UnsafeCell<std::mem::MaybeUninit<libc::termios>>);

// SAFETY: written only while SINGLY is false, by the reader thread alone; read only by whoever
// swaps SINGLY from true to false, after it was written.
unsafe impl Sync for Saved {}

static SAVED: Saved = Saved(std::cell::UnsafeCell::new(std::mem::MaybeUninit::uninit()));

/// Whether the terminal takes keys singly now, as this process set it: then SAVED is how it was.
static SINGLY: AtomicBool = AtomicBool::new(false);

/// Have the terminal take keys one at a time without echoing them, keeping the rest as it is;
/// true once it does.
fn take_keys_singly() -> bool {
    if SINGLY.load(Ordering::SeqCst) {
        return true;
    }
    // SAFETY: SINGLY is false, so nothing reads SAVED, and only this thread writes it.
    unsafe {
        let saved = &mut *SAVED.0.get();
        if libc::tcgetattr(STDIN, saved.as_mut_ptr()) != 0 {
            return false;
        }
        let mut singly = saved.assume_init();
        singly.c_lflag &= !(libc::ICANON | libc::ECHO);
        singly.c_cc[libc::VMIN] = 1;
        singly.c_cc[libc::VTIME] = 0;
        SINGLY.store(true, Ordering::SeqCst);
        if libc::tcsetattr(STDIN, libc::TCSANOW, &singly) != 0 {
            SINGLY.store(false, Ordering::SeqCst);
            return false;
        }
    }
    true
}

/// Put the terminal back as it was, if this process changed it.
fn put_back() {
    if SINGLY.swap(false, Ordering::SeqCst) {
        // SAFETY: SINGLY was true, so SAVED was written.
        unsafe { libc::tcsetattr(STDIN, libc::TCSANOW, (*SAVED.0.get()).as_ptr()) };
    }
}

/// Put the terminal back before a panic says what happened, whatever happens to the process after.
fn install_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let before = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic| {
            put_back();
            before(panic);
        }));
    });
}

/// The pipe SIGTSTP's handler tells the reader on: its two ends, -1 until it's made.
struct Pipe {
    read: AtomicI32,
    write: AtomicI32,
}

static SUSPEND: Pipe = Pipe {
    read: AtomicI32::new(-1),
    write: AtomicI32::new(-1),
};

/// On Ctrl-Z (SIGTSTP), the reader puts the terminal back before the process stops, as a stopped
/// job leaves it to its shell, and takes keys singly again once it's in the foreground. What
/// SIGTSTP did before.
fn on_suspend() -> Option<libc::sigaction> {
    if SUSPEND.read.load(Ordering::SeqCst) < 0 {
        let mut ends = [0; 2];
        // SAFETY: two fds, ours; each end is kept for as long as the process runs.
        unsafe {
            if libc::pipe(ends.as_mut_ptr()) != 0 {
                return None;
            }
            for end in ends {
                libc::fcntl(end, libc::F_SETFD, libc::FD_CLOEXEC);
                libc::fcntl(end, libc::F_SETFL, libc::O_NONBLOCK);
            }
        }
        SUSPEND.write.store(ends[1], Ordering::SeqCst);
        SUSPEND.read.store(ends[0], Ordering::SeqCst);
    }
    // SAFETY: a handler of ours for SIGTSTP; `was` is ours to fill.
    unsafe {
        let mut was: libc::sigaction = std::mem::zeroed();
        libc::sigaction(libc::SIGTSTP, &handling_suspend(), &mut was);
        Some(was)
    }
}

/// SIGTSTP handled by `suspended`.
fn handling_suspend() -> libc::sigaction {
    // SAFETY: a sigaction is plain data; zeroed, then filled in.
    unsafe {
        let mut now: libc::sigaction = std::mem::zeroed();
        now.sa_sigaction = suspended as extern "C" fn(libc::c_int) as libc::sighandler_t;
        libc::sigemptyset(&mut now.sa_mask);
        now.sa_flags = libc::SA_RESTART;
        now
    }
}

/// SIGTSTP's handler: tell the reader, which suspends the process. Only what's safe in a signal
/// handler: a write.
extern "C" fn suspended(_: libc::c_int) {
    let byte = 0u8;
    // SAFETY: one byte, from ours, to a pipe that's never closed.
    unsafe {
        libc::write(
            SUSPEND.write.load(Ordering::SeqCst),
            (&raw const byte).cast(),
            1,
        )
    };
}

/// Suspend the process as Ctrl-Z does, with the terminal put back first; handling Ctrl-Z again
/// once continued (`fg`, `bg`).
fn suspend() {
    let mut told = [0u8; 16];
    // SAFETY: into ours; the pipe doesn't block.
    while unsafe {
        libc::read(
            SUSPEND.read.load(Ordering::SeqCst),
            told.as_mut_ptr().cast(),
            16,
        )
    } > 0
    {}
    put_back();
    // SAFETY: SIGTSTP's default stops the process, which raise does, here; then it's ours again.
    unsafe {
        libc::signal(libc::SIGTSTP, libc::SIG_DFL);
        libc::raise(libc::SIGTSTP); // stopped here, until continued
        libc::sigaction(libc::SIGTSTP, &handling_suspend(), std::ptr::null_mut());
    }
}

//! The keys a Host run in a terminal reads: `q` quits and `d` detaches (#97). Only while it's the
//! terminal's foreground job: started with `&`, or with no terminal, it reads nothing, and while
//! it's in the background it reads nothing, so its shell never stops it for reading (ADR 0022:
//! reading a key isn't asking a question).
//!
//! While it reads, the terminal takes keys one at a time and doesn't echo them; everything else
//! is as it was, so Ctrl-C and Ctrl-Z still signal, and lines printed meanwhile still start at the
//! left. A key counts only pressed on its own: an arrow key's escape sequence, a capital, or pasted
//! text does nothing. The terminal is put back as it was however the Host ends (a key, a signal,
//! an error, a panic), and whenever it's suspended (Ctrl-Z): `fg` brings the keys back.
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
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
    // SAFETY: isatty, tcgetpgrp and getpgrp take no pointers, and only ask.
    unsafe { libc::isatty(STDIN) == 1 && libc::tcgetpgrp(STDIN) == libc::getpgrp() }
}

const STDIN: libc::c_int = libc::STDIN_FILENO;

/// Reads the Host's keys from its terminal, while it's the foreground job, until dropped.
pub struct Keys {
    keys: mpsc::UnboundedReceiver<Key>,
    done: Arc<AtomicBool>,
    reader: Option<std::thread::JoinHandle<()>>,
    /// What Ctrl-Z did before, put back when done.
    ctrl_z_was: Option<libc::sigaction>,
}

impl Keys {
    /// Start reading keys, showing `hint` the first time they're read; None with no terminal on
    /// standard input.
    pub fn read(hint: &'static str) -> Option<Keys> {
        // SAFETY: isatty takes no pointers, and only asks.
        if unsafe { libc::isatty(STDIN) } != 1 {
            return None;
        }
        terminal::put_back_on_panic();
        ctrl_z::forget_pending();
        let (send, keys) = mpsc::unbounded_channel();
        let done = Arc::new(AtomicBool::new(false));
        let reading = done.clone();
        let reader = std::thread::Builder::new()
            .name("keys".into())
            .spawn(move || read_keys(&send, &reading, hint))
            .ok()?;
        // Once there's a reader to suspend the process: until then, Ctrl-Z does as it always did.
        let ctrl_z_was = ctrl_z::handle();
        Some(Keys {
            keys,
            done,
            reader: Some(reader),
            ctrl_z_was,
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
        terminal::put_back();
        if let Some(was) = &self.ctrl_z_was {
            ctrl_z::restore(was);
        }
        ctrl_z::forget_pending();
    }
}

/// How long the reader waits for a key before it looks again at whether it's still to read.
const LOOK_AGAIN: Duration = Duration::from_millis(100);

/// How soon after an escape sequence's first bytes the rest of it comes, at most: what comes
/// sooner is taken as part of it.
const ESCAPE_SEQUENCE: Duration = Duration::from_millis(30);

/// Read keys until `done`, while in the foreground, showing `hint` the first time; the terminal
/// as it was once it's done. The reader alone takes keys singly, and a Ctrl-Z comes to it too
/// (`ctrl_z`), so the terminal is put back before the process stops.
///
/// It blocks SIGTTIN and SIGTTOU for itself, so the shell never stops this job for what it does:
/// a read from the background fails instead (EIO), and changing the terminal goes ahead. It only
/// changes the terminal in the foreground; but the shell may take the terminal back between
/// looking and changing it, and then the change lands on the shell's terminal. That window, a
/// few instructions wide, remains.
fn read_keys(send: &mpsc::UnboundedSender<Key>, done: &AtomicBool, hint: &str) {
    block_job_control_signals();
    let mut hinted = false;
    while !done.load(Ordering::SeqCst) {
        let reading = in_the_foreground() && terminal::take_keys_singly();
        if reading && !hinted {
            eprintln!("{hint}");
            hinted = true;
        }
        let mut ready = [
            libc::pollfd {
                fd: ctrl_z::told(),
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
        // SAFETY: `ready` holds `watched` pollfds, and outlives the call. A negative fd (no Ctrl-Z
        // pipe) is skipped by poll.
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
            ctrl_z::suspend();
            continue;
        }
        if ready[1].revents & libc::POLLIN == 0 {
            std::thread::sleep(LOOK_AGAIN); // hung up: nothing more to read
            continue;
        }
        let Some(chunk) = read_chunk() else {
            std::thread::sleep(LOOK_AGAIN);
            continue;
        };
        if chunk.contains(&ESC) {
            skip_escape_sequence();
            continue;
        }
        let key = match chunk.as_slice() {
            [b'q'] => Key::Quit,
            [b'd'] => Key::Detach,
            _ => continue, // anything else, or more than one key at once (pasted)
        };
        if send.send(key).is_err() {
            break;
        }
    }
    terminal::put_back();
}

const ESC: u8 = 0x1b;

/// What's there to read on the terminal, up to a few bytes; None when nothing could be read
/// (in the background, it fails rather than stopping the job: SIGTTIN is blocked).
fn read_chunk() -> Option<Vec<u8>> {
    let mut chunk = [0u8; 64];
    // SAFETY: reads at most chunk.len() bytes into chunk, which outlives the call.
    let read = unsafe { libc::read(STDIN, chunk.as_mut_ptr().cast(), chunk.len()) };
    let read = usize::try_from(read).ok().filter(|&n| n > 0)?;
    Some(chunk[..read].to_vec())
}

/// Read and ignore what follows an escape sequence's first bytes within `ESCAPE_SEQUENCE`: the
/// rest of it, when it came in pieces.
fn skip_escape_sequence() {
    loop {
        let mut ready = libc::pollfd {
            fd: STDIN,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one pollfd, which outlives the call.
        let polled =
            unsafe { libc::poll(&mut ready, 1, ESCAPE_SEQUENCE.as_millis() as libc::c_int) };
        if polled <= 0 || ready.revents & libc::POLLIN == 0 || read_chunk().is_none() {
            return;
        }
    }
}

/// Block SIGTTIN and SIGTTOU for this thread alone.
fn block_job_control_signals() {
    // SAFETY: `set` is ours, initialised by sigemptyset before use; pthread_sigmask changes only
    // this thread's mask.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGTTIN);
        libc::sigaddset(&mut set, libc::SIGTTOU);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
    }
}

/// The terminal's settings: taking keys singly, and putting the terminal back as it was.
mod terminal {
    use super::{STDIN, in_the_foreground};
    use std::sync::{Mutex, MutexGuard, TryLockError};

    /// The terminal as it was before keys were taken singly; Some while they are.
    static BEFORE: Mutex<Option<libc::termios>> = Mutex::new(None);

    fn before() -> MutexGuard<'static, Option<libc::termios>> {
        BEFORE.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Have the terminal take keys one at a time without echoing them, keeping the rest as it
    /// is; true once it does. Only in the foreground.
    pub(super) fn take_keys_singly() -> bool {
        let mut before = before();
        if before.is_some() {
            return true;
        }
        // SAFETY: a termios is plain data, filled in by tcgetattr before it's read.
        let mut was: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: tcgetattr writes only `was`, which outlives the call.
        if unsafe { libc::tcgetattr(STDIN, &mut was) } != 0 {
            return false;
        }
        let mut singly = was;
        singly.c_lflag &= !(libc::ICANON | libc::ECHO);
        singly.c_cc[libc::VMIN] = 1;
        singly.c_cc[libc::VTIME] = 0;
        if !set(&singly) {
            return false;
        }
        *before = Some(was);
        true
    }

    /// Put the terminal back as it was, if keys are taken singly.
    pub(super) fn put_back() {
        put_back_from(before());
    }

    fn put_back_from(mut before: MutexGuard<'_, Option<libc::termios>>) {
        if let Some(was) = before.take() {
            set(&was);
        }
    }

    /// Set the terminal to `to`, with SIGTTOU blocked for this thread so its shell never stops
    /// the job for it; only in the foreground, since in the background the terminal is its
    /// shell's, which sets it as it needs. True once it's set.
    fn set(to: &libc::termios) -> bool {
        if !in_the_foreground() {
            return false;
        }
        // SAFETY: `ttou` and `mask` are ours, initialised before use; pthread_sigmask changes
        // only this thread's mask, and it's put back as it was; tcsetattr reads only `to`.
        unsafe {
            let mut ttou: libc::sigset_t = std::mem::zeroed();
            let mut mask: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut ttou);
            libc::sigaddset(&mut ttou, libc::SIGTTOU);
            libc::pthread_sigmask(libc::SIG_BLOCK, &ttou, &mut mask);
            let set = libc::tcsetattr(STDIN, libc::TCSANOW, to) == 0;
            libc::pthread_sigmask(libc::SIG_SETMASK, &mask, std::ptr::null_mut());
            set
        }
    }

    /// Put the terminal back before a panic says what happened, whatever happens to the process
    /// after: unless the panic is on a thread changing it, which then can't finish.
    pub(super) fn put_back_on_panic() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let then = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |panic| {
                match BEFORE.try_lock() {
                    Ok(before) => put_back_from(before),
                    Err(TryLockError::Poisoned(e)) => put_back_from(e.into_inner()),
                    Err(TryLockError::WouldBlock) => {}
                }
                then(panic);
            }));
        });
    }
}

/// Ctrl-Z (SIGTSTP) while keys are read: its handler only tells the reader, over a pipe, and the
/// reader puts the terminal back and suspends the process, so nothing changes the terminal while
/// it's being suspended.
mod ctrl_z {
    use std::sync::atomic::{AtomicI32, Ordering};

    /// The pipe the handler tells the reader on: its ends, -1 until it's made. Made once, and
    /// never closed.
    static TOLD: AtomicI32 = AtomicI32::new(-1);
    static TELL: AtomicI32 = AtomicI32::new(-1);

    /// The end the reader waits on; -1 if there's no pipe.
    pub(super) fn told() -> i32 {
        TOLD.load(Ordering::SeqCst)
    }

    /// Handle Ctrl-Z from now on; what it did before. None if the pipe couldn't be made, when
    /// Ctrl-Z goes on as it was.
    pub(super) fn handle() -> Option<libc::sigaction> {
        if told() < 0 {
            let mut ends = [0; 2];
            // SAFETY: pipe writes two fds into `ends`; fcntl takes no pointers. Both ends are kept
            // for as long as the process runs.
            unsafe {
                if libc::pipe(ends.as_mut_ptr()) != 0 {
                    return None;
                }
                for end in ends {
                    libc::fcntl(end, libc::F_SETFD, libc::FD_CLOEXEC);
                    libc::fcntl(end, libc::F_SETFL, libc::O_NONBLOCK);
                }
            }
            TELL.store(ends[1], Ordering::SeqCst);
            TOLD.store(ends[0], Ordering::SeqCst);
        }
        // SAFETY: sigaction reads `on_ctrl_z()` and writes `was`, both ours and initialised.
        unsafe {
            let mut was: libc::sigaction = std::mem::zeroed();
            libc::sigaction(libc::SIGTSTP, &on_ctrl_z(), &mut was);
            Some(was)
        }
    }

    /// Have Ctrl-Z do what `was` says again.
    pub(super) fn restore(was: &libc::sigaction) {
        // SAFETY: `was` is what sigaction gave back for SIGTSTP.
        unsafe { libc::sigaction(libc::SIGTSTP, was, std::ptr::null_mut()) };
    }

    /// SIGTSTP handled by `tell_the_reader`.
    fn on_ctrl_z() -> libc::sigaction {
        // SAFETY: a sigaction is plain data: zeroed, then filled in.
        unsafe {
            let mut now: libc::sigaction = std::mem::zeroed();
            now.sa_sigaction = tell_the_reader as extern "C" fn(libc::c_int) as libc::sighandler_t;
            libc::sigemptyset(&mut now.sa_mask);
            now.sa_flags = libc::SA_RESTART;
            now
        }
    }

    /// SIGTSTP's handler: tell the reader, which suspends the process. Only what's safe in a
    /// signal handler: a write, with errno as it was after.
    extern "C" fn tell_the_reader(_: libc::c_int) {
        let byte = 0u8;
        // SAFETY: errno is this thread's, and put back as it was; write reads one byte of ours.
        // A full pipe (Ctrl-Z pressed often, unread) loses only a byte saying the same.
        unsafe {
            let was = *errno();
            libc::write(TELL.load(Ordering::SeqCst), (&raw const byte).cast(), 1);
            *errno() = was;
        }
    }

    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
    unsafe fn errno() -> *mut libc::c_int {
        // SAFETY: the thread's own errno.
        unsafe { libc::__error() }
    }

    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "freebsd")))]
    unsafe fn errno() -> *mut libc::c_int {
        // SAFETY: the thread's own errno.
        unsafe { libc::__errno_location() }
    }

    /// Forget a Ctrl-Z told and not acted on: one from before, or after, keys are read.
    pub(super) fn forget_pending() {
        let fd = told();
        if fd < 0 {
            return;
        }
        let mut pending = [0u8; 16];
        // SAFETY: reads at most 16 bytes into `pending`; the pipe never blocks.
        while unsafe { libc::read(fd, pending.as_mut_ptr().cast(), pending.len()) } > 0 {}
    }

    /// Suspend the process as Ctrl-Z does, with the terminal put back first; handling Ctrl-Z
    /// again once it's continued (`fg`, `bg`).
    pub(super) fn suspend() {
        forget_pending();
        super::terminal::put_back();
        // SAFETY: SIGTSTP's default stops the process, which raise has it do, here; then it's
        // handled again. sigaction reads `on_ctrl_z()`, ours.
        unsafe {
            libc::signal(libc::SIGTSTP, libc::SIG_DFL);
            libc::raise(libc::SIGTSTP); // stopped here, until continued
            libc::sigaction(libc::SIGTSTP, &on_ctrl_z(), std::ptr::null_mut());
        }
    }
}

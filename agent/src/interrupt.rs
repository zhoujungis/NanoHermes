//! Ctrl-C takeover: a SIGINT handler that flips a flag the agent loop polls.
//!
//! While a turn is running, SIGINT (Ctrl-C) sets a flag instead of killing the
//! process, so the current turn aborts gracefully and control returns to the
//! REPL with the conversation intact. The handler is installed only for the
//! duration of a turn: while idle the default disposition applies, so Ctrl-C
//! still exits the shell as usual.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);
/// Nesting depth of active turns; the handler is installed at 0 -> 1 and
/// removed at 1 -> 0.
static DEPTH: AtomicUsize = AtomicUsize::new(0);

/// The signal handler is only reachable through the Linux `imp` module, so it
/// is compiled out on other platforms (where the OS owns Ctrl-C handling).
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
extern "C" fn on_sigint(_sig: i32) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

/// True if an interrupt was requested since the last [`take`].
#[cfg(test)]
pub fn triggered() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

/// Consume a pending interrupt flag, returning whether one was set.
pub fn take() -> bool {
    INTERRUPTED.swap(false, Ordering::SeqCst)
}

/// Request an interrupt from inside the process. Embedders (the desktop GUI)
/// have no way to deliver SIGINT to themselves, so they flip the same flag
/// directly; the agent loop picks it up at the next step boundary.
pub fn request() {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

/// Install the takeover handler for SIGINT (ref-counted; safe to nest).
/// Returns true on success.
pub fn install() -> bool {
    if DEPTH.fetch_add(1, Ordering::SeqCst) > 0 {
        return true;
    }
    #[cfg(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    {
        imp::install()
    }
    #[cfg(not(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )))]
    {
        false
    }
}

/// Undo one [`install`]; when the outermost turn ends this restores the default
/// SIGINT disposition (Ctrl-C terminates the process).
pub fn restore_default() {
    // Saturating: never let an unbalanced leave drop below zero.
    let prev = DEPTH.load(Ordering::SeqCst);
    if prev == 0 {
        return;
    }
    // Only the outermost leave restores the default SIGINT disposition.
    if DEPTH.fetch_sub(1, Ordering::SeqCst) == 1 {
        #[cfg(all(
            target_os = "linux",
            any(target_arch = "x86_64", target_arch = "aarch64")
        ))]
        imp::restore_default();
    }
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod imp {
    use super::on_sigint;
    use crate::sys::{nr, syscall};

    const SIGINT: usize = 2;
    const SA_RESTART: usize = 0x1000_0000;
    #[cfg(target_arch = "x86_64")]
    const SA_RESTORER: usize = 0x0400_0000;
    const SIG_DFL: usize = 0;
    /// `_NSIG / 8`, the size the kernel expects for `sigset_t`.
    const SIGSET_SIZE: usize = 8;

    #[repr(C)]
    struct SigAction {
        handler: usize,
        flags: usize,
        restorer: usize,
        mask: u64,
    }

    // On x86_64 the kernel refuses to deliver a caught signal unless the caller
    // supplies a restorer that invokes `rt_sigreturn` (Linux syscall 15). This
    // is the only reason the module emits assembly.
    #[cfg(target_arch = "x86_64")]
    mod restorer {
        core::arch::global_asm!(
            ".globl nanohermes_sigrestorer",
            ".hidden nanohermes_sigrestorer",
            "nanohermes_sigrestorer:",
            "mov rax, 15",
            "syscall",
        );
        extern "C" {
            pub fn nanohermes_sigrestorer();
        }
    }

    fn install_action(handler: usize, flags: usize, restorer: usize) -> bool {
        let act = SigAction {
            handler,
            flags,
            restorer,
            mask: 0,
        };
        let r = unsafe {
            syscall(
                nr::RT_SIGACTION,
                SIGINT,
                &act as *const _ as usize,
                0,
                SIGSET_SIZE,
                0,
                0,
            )
        };
        r == 0
    }

    pub fn install() -> bool {
        let handler: extern "C" fn(i32) = on_sigint;
        #[cfg(target_arch = "x86_64")]
        let (flags, restorer) = (
            SA_RESTART | SA_RESTORER,
            restorer::nanohermes_sigrestorer as usize,
        );
        #[cfg(target_arch = "aarch64")]
        let (flags, restorer) = (SA_RESTART, 0usize);
        install_action(handler as usize, flags, restorer)
    }

    pub fn restore_default() {
        install_action(SIG_DFL, 0, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sigint_handler_sets_and_clears_flag() {
        assert!(!triggered());
        if !install() {
            eprintln!("skipping: could not install SIGINT handler");
            return;
        }
        // Deliver SIGINT to this process and wait for the handler to run.
        #[cfg(all(
            target_os = "linux",
            any(target_arch = "x86_64", target_arch = "aarch64")
        ))]
        {
            let pid = std::process::id() as usize;
            unsafe {
                crate::sys::syscall(crate::sys::nr::KILL, pid, 2, 0, 0, 0, 0);
            }
        }
        for _ in 0..200 {
            if triggered() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(triggered(), "SIGINT handler did not set the flag");
        assert!(take(), "take should report the pending interrupt");
        assert!(!triggered(), "take should clear the flag");
        restore_default();
    }
}

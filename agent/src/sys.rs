//! Minimal raw Linux syscall shims.
//!
//! The agent deliberately avoids a libc dependency, but two features need
//! syscalls the Rust standard library does not expose: the Landlock sandbox and
//! the Ctrl-C takeover handler. Both reach the kernel through this module.
#![cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]

#[cfg(target_arch = "x86_64")]
pub mod nr {
    pub const CLOSE: usize = 3;
    pub const RT_SIGACTION: usize = 13;
    #[cfg(test)]
    pub const KILL: usize = 62;
    pub const OPENAT: usize = 257;
    pub const PRCTL: usize = 157;
}

#[cfg(target_arch = "aarch64")]
pub mod nr {
    pub const CLOSE: usize = 57;
    pub const RT_SIGACTION: usize = 134;
    #[cfg(test)]
    pub const KILL: usize = 129;
    pub const OPENAT: usize = 56;
    pub const PRCTL: usize = 167;
}

/// Invoke a raw syscall with up to six arguments.
///
/// # Safety
/// The caller must supply a syscall number and arguments valid for this
/// architecture; an invalid combination can corrupt the process.
#[inline]
pub unsafe fn syscall(
    n: usize,
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
    a5: usize,
    a6: usize,
) -> isize {
    let ret: isize;
    #[cfg(target_arch = "x86_64")]
    core::arch::asm!(
        "syscall",
        inlateout("rax") n as isize => ret,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        in("r10") a4,
        in("r8") a5,
        in("r9") a6,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack, preserves_flags)
    );
    #[cfg(target_arch = "aarch64")]
    core::arch::asm!(
        "svc 0",
        in("x8") n,
        inlateout("x0") a1 => ret,
        in("x1") a2,
        in("x2") a3,
        in("x3") a4,
        in("x4") a5,
        in("x5") a6,
        options(nostack)
    );
    ret
}

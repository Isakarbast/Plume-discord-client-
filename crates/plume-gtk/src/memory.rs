//! Keeping memory use honest.
//!
//! glibc's malloc keeps freed memory around for reuse, and gives every busy
//! thread its own arena. For a chat client that's mostly waste: READY alone
//! can allocate tens of megabytes that are freed a second later but stay in
//! our RSS. So: few arenas, and hand free pages back to the kernel now and
//! then.

/// Call once, before any threads start.
pub fn tune() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: mallopt only changes allocator settings; called before other threads exist.
    unsafe {
        libc::mallopt(libc::M_ARENA_MAX, 2);
    }
}

/// Return freed heap pages to the kernel. Cheap enough to run every minute.
pub fn give_back() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: malloc_trim is safe to call at any time from any thread.
    unsafe {
        libc::malloc_trim(0);
    }
}

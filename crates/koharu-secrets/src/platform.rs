#[cfg(any(target_os = "linux", all(target_os = "macos", debug_assertions)))]
#[path = "platform/file.rs"]
mod implementation;

#[cfg(not(any(target_os = "linux", all(target_os = "macos", debug_assertions))))]
#[path = "platform/native.rs"]
mod implementation;

pub(crate) use implementation::configure;

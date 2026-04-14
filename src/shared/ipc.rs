// Simple Unix-domain-socket based single-instance IPC skeleton.
#[cfg(unix)]
pub fn try_acquire_instance_lock(_name: &str) -> Result<bool, String> {
    // Placeholder: implement Unix domain socket bind/check
    Ok(true)
}

#[cfg(not(unix))]
pub fn try_acquire_instance_lock(_name: &str) -> Result<bool, String> {
    Err("IPC single-instance not implemented on this platform in skeleton".into())
}

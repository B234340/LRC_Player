use crate::infrastructure::audio::cpal_output;

/// Placeholder for device watcher that observes hotplug events and notifies clients.
/// Currently provides device enumeration helpers and can be extended for hotplug events.
pub struct DeviceWatcher;

impl DeviceWatcher {
    pub fn start() -> Self {
        DeviceWatcher
    }

    pub fn available_devices() -> Vec<String> {
        cpal_output::available_device_names()
    }

    pub fn default_device() -> Option<String> {
        cpal_output::default_device_name()
    }
}

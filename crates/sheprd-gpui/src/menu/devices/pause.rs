//! Pausing a saved device: it stays listed in the sidebar, greyed, but
//! nothing connects to it, so it neither shows its workspaces nor notifies,
//! until it is resumed. The device's own Herdr and terminals keep running.
//! SSH devices keep this as their catalog profile's `enabled` flag, which
//! `herdr machine enable|disable` writes; cloud devices in the app's own list.
use super::setup;
use crate::{SheprdWindow, i18n::fl, window::Flash};
use gpui::Context;
use sheprd_client::ConnectTarget;

/// What a pause changes, read off the endpoint before it runs in the
/// background.
enum Pausable {
    Ssh(String),
    #[cfg(feature = "cloud")]
    Cloud {
        provider: crate::cloud::CloudProvider,
        id: String,
    },
}

impl SheprdWindow {
    /// Whether the saved device `id` is paused. Local and unknown devices
    /// never are.
    pub(crate) fn device_paused(&self, id: &str) -> bool {
        self.endpoints[1..]
            .iter()
            .any(|endpoint| endpoint.id == id && !endpoint.enabled)
    }

    /// Whether the device `id` is a saved one a pause applies to: an SSH or
    /// cloud device, while the catalog is the one device setup writes.
    pub(crate) fn device_pausable(&self, id: &str) -> bool {
        self.pausable(id).is_some()
    }

    fn pausable(&self, id: &str) -> Option<Pausable> {
        if self.device_setup_unavailable().is_some() {
            return None;
        }
        let endpoint = self.endpoints[1..]
            .iter()
            .find(|endpoint| endpoint.id == id)?;
        match endpoint.saved_target() {
            ConnectTarget::Ssh { .. } => {
                crate::endpoint::saved_profile_id(id).map(|profile| Pausable::Ssh(profile.into()))
            }
            #[cfg(feature = "cloud")]
            ConnectTarget::Cloud { provider, id, .. } => Some(Pausable::Cloud {
                provider: *provider,
                id: id.clone(),
            }),
            _ => None,
        }
    }

    /// Pauses (`paused`) or resumes the saved device `id`, in the background.
    /// Every window's catalog watcher then stops or restarts its connection.
    /// Returns false, changing nothing, for a device that cannot be paused.
    pub(crate) fn set_device_paused(
        &mut self,
        id: &str,
        paused: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(pausable) = self.pausable(id) else {
            return false;
        };
        let label = self
            .endpoints
            .iter()
            .find(|endpoint| endpoint.id == id)
            .map(|endpoint| endpoint.label.clone())
            .unwrap_or_default();
        tracing::info!(category = "device_pause", id, paused, "Pausing device");
        let background = cx.background_executor().spawn(async move {
            match pausable {
                Pausable::Ssh(profile) => setup::set_enabled(&profile, !paused),
                #[cfg(feature = "cloud")]
                Pausable::Cloud { provider, id } => {
                    crate::cloud::set_enabled(provider, &id, !paused).map_err(Into::into)
                }
            }
        });
        cx.spawn(async move |this, cx| {
            let result = background.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(()) => {
                    this.catalog.refresh();
                    cx.notify();
                }
                Err(error) => {
                    tracing::warn!(category = "device_pause", "Device pause failed");
                    this.show_flash(
                        Flash::warning(fl!(
                            "host-menu-pause-failed",
                            device = label,
                            error = error.to_string(),
                        )),
                        cx,
                    );
                }
            });
        })
        .detach();
        true
    }
}

#[cfg(test)]
mod tests;

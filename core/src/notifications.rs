//! Desktop notifications for events the user would otherwise miss while the
//! app is unfocused. Platform-specific senders: notify-send (Linux),
//! osascript (macOS), PowerShell toast (Windows).

use std::collections::HashMap;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tracing::warn;

use crate::bridge::NetworkEvent;

const NOTIFICATION_DEDUP_WINDOW: Duration = Duration::from_secs(30);

pub trait NotificationSender: Send + Sync {
    fn send(&self, summary: &str, body: &str) -> Result<()>;
}

struct SystemNotificationSender;

impl NotificationSender for SystemNotificationSender {
    fn send(&self, summary: &str, body: &str) -> Result<()> {
        send_system_notification(summary, body)
    }
}

#[cfg(target_os = "linux")]
fn send_system_notification(summary: &str, body: &str) -> Result<()> {
    let status = Command::new("notify-send")
        .arg(summary)
        .arg(body)
        .status()
        .context("failed to run notify-send")?;
    anyhow::ensure!(status.success(), "notify-send exited with {status}");
    Ok(())
}

#[cfg(target_os = "macos")]
fn send_system_notification(summary: &str, body: &str) -> Result<()> {
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        escape_applescript_string(body),
        escape_applescript_string(summary),
    );
    let status = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .status()
        .context("failed to run osascript")?;
    anyhow::ensure!(status.success(), "osascript exited with {status}");
    Ok(())
}

#[cfg(target_os = "windows")]
fn send_system_notification(summary: &str, body: &str) -> Result<()> {
    let script = format!(
        "$xml = New-Object Windows.Data.Xml.Dom.XmlDocument; \
         $template = '<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>'; \
         $xml.LoadXml($template); \
         $toast = New-Object Windows.UI.Notifications.ToastNotification $xml; \
         [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('jyn').Show($toast);",
        escape_xml(summary),
        escape_xml(body),
    );
    let status = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status()
        .context("failed to run powershell for toast notification")?;
    anyhow::ensure!(status.success(), "powershell exited with {status}");
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn send_system_notification(_summary: &str, _body: &str) -> Result<()> {
    anyhow::bail!("system notifications are not supported on this platform");
}

#[cfg(target_os = "macos")]
fn escape_applescript_string(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(target_os = "windows")]
fn escape_xml(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub struct NotificationState {
    sender: Box<dyn NotificationSender>,
    warned_unavailable: bool,
    recently_sent: HashMap<String, Instant>,
}

impl Default for NotificationState {
    fn default() -> Self {
        Self {
            sender: Box::new(SystemNotificationSender),
            warned_unavailable: false,
            recently_sent: HashMap::new(),
        }
    }
}

impl NotificationState {
    pub fn on_event(&mut self, event: &NetworkEvent, app_focused: bool, now: Instant) {
        if app_focused {
            return;
        }

        // Friend requests, hearts and comments on own posts become
        // notifications in later milestones; today only new pending
        // friendship requests are notable enough.
        if let NetworkEvent::LocalStateUpdated { state } = event {
            for request in &state.pending_requests {
                let body = format!(
                    "{} would like to be friends",
                    request.requester_display_name
                );
                self.send_notification("Friendship request", &body, now);
            }
        }
    }

    fn send_notification(&mut self, summary: &str, body: &str, now: Instant) {
        let dedup_key = format!("{summary}\u{1f}{body}");
        self.recently_sent
            .retain(|_, sent_at| now.duration_since(*sent_at) < NOTIFICATION_DEDUP_WINDOW);
        if self.recently_sent.contains_key(&dedup_key) {
            return;
        }
        self.recently_sent.insert(dedup_key, now);

        if let Err(err) = self.sender.send(summary, body) {
            if !self.warned_unavailable {
                warn!("failed to send system notification: {err:#}");
                self.warned_unavailable = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;
    use crate::domain::{PendingFriendRequest, ReducedProfileState, Visibility};

    struct CountingSender(Arc<AtomicUsize>);

    impl NotificationSender for CountingSender {
        fn send(&self, _summary: &str, _body: &str) -> Result<()> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn state_with_request() -> ReducedProfileState {
        ReducedProfileState {
            profile_id: "me".into(),
            display_name: None,
            bio: String::new(),
            default_visibility: Visibility::Friends,
            default_lifetime_secs: None,
            posts: Vec::new(),
            followed_profile_ids: Vec::new(),
            hearts: Vec::new(),
            comments: Vec::new(),
            pending_requests: vec![PendingFriendRequest {
                requester_profile_id: "wen".into(),
                requester_display_name: "Wen Li".into(),
                greeting: None,
                recorded_at: 10,
            }],
            tombstoned_post_ids: Vec::new(),
        }
    }

    #[test]
    fn pending_requests_notify_once_while_unfocused() {
        let count = Arc::new(AtomicUsize::new(0));
        let mut notifications = NotificationState {
            sender: Box::new(CountingSender(count.clone())),
            warned_unavailable: false,
            recently_sent: HashMap::new(),
        };
        let event = NetworkEvent::LocalStateUpdated {
            state: state_with_request(),
        };
        let now = Instant::now();

        // Focused: no notification. Unfocused: one, deduped on repeat.
        notifications.on_event(&event, true, now);
        assert_eq!(count.load(Ordering::SeqCst), 0);
        notifications.on_event(&event, false, now);
        notifications.on_event(&event, false, now);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}

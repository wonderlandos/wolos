//! org.freedesktop.Notifications on a private dbus-daemon: NotificationClosed reasons (spec: 1 expired, 2 dismissed, 3 CloseNotification).
use shell::ui::notify::{Bus, CloseReason, Event};
use std::collections::HashMap;
use std::io::BufRead;
use zbus::zvariant::Value;

struct Daemon(std::process::Child, String);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn daemon() -> Option<Daemon> {
    let mut c = std::process::Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address=1"])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    let mut line = String::new();
    std::io::BufReader::new(c.stdout.take()?)
        .read_line(&mut line)
        .ok()?;
    Some(Daemon(c, line.trim().to_string()))
}

#[test]
fn notification_closed_reasons() {
    let Some(d) = daemon() else {
        eprintln!("skipped: no dbus-daemon");
        return;
    };
    async_io::block_on(async {
        let (tx, rx) = async_channel::unbounded();
        let bus = Bus::serve(Some(&d.1), tx).await.expect("serve");
        let client = zbus::connection::Builder::address(d.1.as_str())
            .unwrap()
            .build()
            .await
            .unwrap();
        let p = zbus::Proxy::new(
            &client,
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
        )
        .await
        .unwrap();
        let mut closed = p.receive_signal("NotificationClosed").await.unwrap();
        let notify = |summary: &'static str| {
            let p = p.clone();
            async move {
                let hints: HashMap<&str, Value> = HashMap::new();
                let id: u32 = p
                    .call(
                        "Notify",
                        &(
                            "app",
                            0u32,
                            "",
                            summary,
                            "",
                            Vec::<&str>::new(),
                            hints,
                            -1i32,
                        ),
                    )
                    .await
                    .unwrap();
                id
            }
        };
        let next = async || {
            use futures_lite::StreamExt;
            let m = closed.next().await.unwrap();
            m.body().deserialize::<(u32, u32)>().unwrap()
        };
        let mut next = next;

        let a = notify("a").await;
        assert!(matches!(rx.recv().await.unwrap(), Event::Show(n) if n.id == a));
        assert!(bus.close(a, CloseReason::Expired).await.unwrap());
        assert_eq!(next().await, (a, 1));
        // Already closed: no second signal.
        assert!(!bus.close(a, CloseReason::Dismissed).await.unwrap());

        let b = notify("b").await;
        rx.recv().await.unwrap();
        assert!(bus.close(b, CloseReason::Dismissed).await.unwrap());
        assert_eq!(next().await, (b, 2));

        let c = notify("c").await;
        rx.recv().await.unwrap();
        let () = p.call("CloseNotification", &(c,)).await.unwrap();
        assert_eq!(next().await, (c, 3));
        assert!(matches!(rx.recv().await.unwrap(), Event::Close(id) if id == c));
    });
}

/// Buttons answer the app: ActionInvoked with the key, then NotificationClosed (2, dismissed).
/// The server says it does actions (notify-send and GLib only send them then).
#[test]
fn actions_are_invoked_then_closed() {
    let Some(d) = daemon() else {
        eprintln!("skipped: no dbus-daemon");
        return;
    };
    async_io::block_on(async {
        use futures_lite::StreamExt;
        let (tx, rx) = async_channel::unbounded();
        let bus = Bus::serve(Some(&d.1), tx).await.expect("serve");
        let client = zbus::connection::Builder::address(d.1.as_str())
            .unwrap()
            .build()
            .await
            .unwrap();
        let p = zbus::Proxy::new(
            &client,
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
        )
        .await
        .unwrap();
        let caps: Vec<String> = p.call("GetCapabilities", &()).await.unwrap();
        assert!(caps.iter().any(|c| c == "actions"), "{caps:?}");
        let mut invoked = p.receive_signal("ActionInvoked").await.unwrap();
        let mut closed = p.receive_signal("NotificationClosed").await.unwrap();
        let hints: HashMap<&str, Value> = HashMap::new();
        let id: u32 = p
            .call(
                "Notify",
                &(
                    "WolOS",
                    0u32,
                    "",
                    "Graphics driver installed",
                    "Restart to finish setting up your graphics card.",
                    vec!["default", "", "restart", "Restart"],
                    hints,
                    -1i32,
                ),
            )
            .await
            .unwrap();
        let Event::Show(n) = rx.recv().await.unwrap() else {
            panic!("no banner");
        };
        assert_eq!(n.actions[1], ("restart".to_string(), "Restart".to_string()));
        assert!(bus.invoke(id, "restart").await.unwrap());
        let m = invoked.next().await.unwrap();
        assert_eq!(
            m.body().deserialize::<(u32, String)>().unwrap(),
            (id, "restart".to_string())
        );
        let m = closed.next().await.unwrap();
        assert_eq!(m.body().deserialize::<(u32, u32)>().unwrap(), (id, 2));
        // Answered once: a second click does nothing.
        assert!(!bus.invoke(id, "restart").await.unwrap());
    });
}

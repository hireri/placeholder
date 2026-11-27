//! Desktop notifications, sent from a thread so a slow notification service never stalls the UI.

pub fn send(title: &str, body: &str) {
    let (title, body) = (title.to_owned(), body.to_owned());
    std::thread::spawn(move || {
        if let Err(e) = notify_rust::Notification::new().appname("Serafin").summary(&title).body(&body).show() {
            eprintln!("notify: {e}");
        }
    });
}

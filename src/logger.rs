// logger.rs

use chrono::Local;

/// Returns a colored version of the given tag.
/// You can adjust the ANSI escape codes for different colors.
pub fn colored_tag(tag: &str) -> String {
    match tag {
        "ENGINE" => format!("\x1b[32;1m[ENGINE]\x1b[0m"), // Bright green
        "WORKER" => format!("\x1b[34;1m[WORKER]\x1b[0m"), // Bright blue
        "MAIN" => format!("\x1b[32;1m[MAIN THREAD]\x1b[0m"),
        _ => format!("[{}]", tag),
    }
}

/// Logs a message with one or more tags along with a timestamp.
/// Usage example:
///
/// ```rust
/// pretty_log!("ENGINE", "WORKER"; "Sync to cold storage every {} seconds", 10);
/// ```
#[macro_export]
macro_rules! pretty_log {
    ( $( $tag:expr ),+ ; $($arg:tt)* ) => {{
        // Build the prefix by converting each tag using `colored_tag`
        let prefix = [$( $crate::logger::colored_tag($tag) ),+].join(" ");
        // Create a timestamp using chrono
        let timestamp = chrono::Local::now().format("[%Y-%m-%d %H:%M:%S]").to_string();
        println!("{} {} {}", timestamp, prefix, format!($($arg)*));
    }};
}

/// Convenience macro for logs that should always show the ENGINE tag.
#[macro_export]
macro_rules! engine_log {
    ( $($arg:tt)* ) => {
        $crate::pretty_log!("ENGINE"; $($arg)*)
    };
}

#[macro_export]
macro_rules! log {
    ( $($arg:tt)* ) => {
        $crate::pretty_log!("MAIN"; $($arg)*)
    };
}

/// Convenience macro for logs that should always show the WORKER tag.
#[macro_export]
macro_rules! worker_log {
    ( $($arg:tt)* ) => {
        $crate::pretty_log!("WORKER"; $($arg)*)
    };
}

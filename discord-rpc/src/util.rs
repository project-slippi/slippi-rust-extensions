use std::{time::{SystemTime, UNIX_EPOCH, Duration}, thread, path::PathBuf};

use directories::BaseDirs;

pub fn current_unix_time() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs().try_into().unwrap()
}

pub fn sleep(millis: u64) {
    thread::sleep(Duration::from_millis(millis));
}

pub fn round(x: f32, decimals: u32) -> f32 {
    let y = 10i32.pow(decimals) as f32;
    (x * y).round() / y
}

pub fn get_appdata_file(suffix: &str) -> Option<PathBuf> {
    if let Some(base_dirs) = BaseDirs::new() {
        return Some(base_dirs.config_dir().join(suffix));
    }
    None
}

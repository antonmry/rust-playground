use std::env;
use std::ffi::CStr;
use std::path::{Path, PathBuf};

fn main() {
    let plugin_path = plugin_path();
    println!("public-cli: looking for plugin at {}", plugin_path.display());

    match load_plugin_message(&plugin_path) {
        Ok(message) => println!("plugin says: {message}"),
        Err(err) => {
            println!("plugin not loaded: {err}");
            println!("running without private feature");
        }
    }
}

fn plugin_path() -> PathBuf {
    if let Some(arg) = env::args().nth(1) {
        return PathBuf::from(arg);
    }

    let mut path = PathBuf::from("plugins");
    path.push(plugin_filename());
    path
}

fn plugin_filename() -> String {
    let base = "private_plugin";
    let prefix = env::consts::DLL_PREFIX;
    let ext = env::consts::DLL_EXTENSION;
    format!("{prefix}{base}.{ext}")
}

fn load_plugin_message(path: &Path) -> Result<String, String> {
    if !path.exists() {
        return Err("plugin file not found".to_string());
    }

    unsafe {
        let lib = libloading::Library::new(path).map_err(|e| e.to_string())?;
        let func: libloading::Symbol<unsafe extern "C" fn() -> *const std::os::raw::c_char> =
            lib.get(b"plugin_message").map_err(|e| e.to_string())?;
        let ptr = func();
        if ptr.is_null() {
            return Err("plugin returned null".to_string());
        }
        let c_str = CStr::from_ptr(ptr);
        let message = c_str.to_string_lossy().into_owned();

        Ok(message)
    }
}

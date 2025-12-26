use std::os::raw::c_char;

#[unsafe(no_mangle)]
pub extern "C" fn plugin_message() -> *const c_char {
    b"hello from the private plugin\0".as_ptr() as *const c_char
}

# Plugin system demo (C ABI)

This workspace shows a minimal runtime plugin setup in Rust using a C ABI.
There are two crates:

- `public-cli`: a public CLI that loads a plugin if a shared library exists.
- `private-plugin`: a private crate compiled as a `cdylib` exposing a C-ABI function.

The CLI checks for the plugin at `plugins/{DLL_PREFIX}private_plugin.{DLL_EXTENSION}`
(e.g. `plugins/libprivate_plugin.so` on Linux, `plugins/libprivate_plugin.dylib`
on macOS, `plugins/private_plugin.dll` on Windows).

## Code walkthrough

### 1) The private plugin (`cdylib`)

`private-plugin/Cargo.toml` declares a `cdylib` crate type so Rust produces a
shared library suitable for dynamic loading:

```toml
[lib]
crate-type = ["cdylib"]
```

`private-plugin/src/lib.rs` exposes a single C-ABI symbol:

```rust
use std::os::raw::c_char;

#[no_mangle]
pub extern "C" fn plugin_message() -> *const c_char {
    b"hello from the private plugin\0".as_ptr() as *const c_char
}
```

Notes:
- `#[no_mangle]` keeps the symbol name stable for dynamic loading.
- `extern "C"` gives the C ABI.
- The returned string is a static, NUL-terminated byte string so the CLI can
  read it safely as a `CStr`.

### 2) The public CLI (runtime loading)

`public-cli/src/main.rs` looks for the plugin and loads the symbol if found:

```rust
fn plugin_filename() -> String {
    let base = "private_plugin";
    let prefix = env::consts::DLL_PREFIX;
    let ext = env::consts::DLL_EXTENSION;
    format!("{prefix}{base}.{ext}")
}
```

This picks the correct shared library name for the current OS.

```rust
unsafe {
    let lib = libloading::Library::new(path).map_err(|e| e.to_string())?;
    let func: libloading::Symbol<unsafe extern "C" fn() -> *const c_char> =
        lib.get(b"plugin_message").map_err(|e| e.to_string())?;
    let ptr = func();
    if ptr.is_null() {
        return Err("plugin returned null".to_string());
    }
    let c_str = CStr::from_ptr(ptr);
    let message = c_str.to_string_lossy().into_owned();

    Ok(message)
}
```

If the plugin is missing, the CLI prints a fallback message and continues.

## Build and run

From the workspace root:

```bash
cargo build
```

Copy the plugin into the expected `plugins/` directory:

```bash
cp target/debug/libprivate_plugin.* plugins/
```

Run the CLI:

```bash
cargo run -p public-cli
```

To simulate the missing-plugin path, move the library out of `plugins/` and
run again. You should see the fallback message.

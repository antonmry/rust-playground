use std::env;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Tell cargo to rerun this script if the eBPF source changes
    println!("cargo:rerun-if-changed=../ebpf-sniffer-ebpf/src");

    // Ensure the eBPF target directory exists
    let target_dir = out_dir.join("../../bpfel-unknown-none");
    std::fs::create_dir_all(&target_dir).ok();
}

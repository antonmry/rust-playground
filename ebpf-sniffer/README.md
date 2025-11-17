# eBPF HTTPS Traffic Sniffer

A high-performance kernel-level packet sniffer written in Rust that captures HTTPS traffic to specific domains using eBPF (Extended Berkeley Packet Filter). This tool attaches to the TC (Traffic Control) egress hook to monitor outgoing traffic on port 443.

## Features

- **Kernel-level packet capture** using eBPF for minimal overhead
- **Domain-based filtering** - specify which domains to monitor
- **TLS/SSL detection** - identifies TLS handshakes and extracts SNI (Server Name Indication)
- **HTTP/2 and gRPC detection** - recognizes modern protocols
- **CSV export** - optionally save captured packets with hex-encoded payloads
- **Multi-CPU support** - processes packets in parallel across all CPU cores
- **Zero packet drops** - packets are always allowed through (monitoring only)

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    Network Interface                     │
│                      (eth0, wlan0)                       │
└─────────────────────────┬───────────────────────────────┘
                          │ Egress Traffic
                          ▼
┌─────────────────────────────────────────────────────────┐
│               TC (Traffic Control) Egress                │
│                    eBPF Classifier                       │
│  ┌────────────────────────────────────────────────────┐ │
│  │  1. Parse Ethernet → IPv4 → TCP headers           │ │
│  │  2. Filter: Port 443 + Target IPs                 │ │
│  │  3. Extract packet payload (up to 1500 bytes)     │ │
│  │  4. Send to userspace via PerfEventArray          │ │
│  └────────────────────────────────────────────────────┘ │
└─────────────────────────┬───────────────────────────────┘
                          │ PerfEventArray
                          ▼
┌─────────────────────────────────────────────────────────┐
│                  Userspace Program                       │
│  ┌────────────────────────────────────────────────────┐ │
│  │  • Resolve domains to IPs                         │ │
│  │  • Load & attach eBPF program                     │ │
│  │  • Process packets from all CPUs                  │ │
│  │  • Analyze: TLS, SNI, HTTP/2, gRPC                │ │
│  │  • Log results + optional CSV export              │ │
│  └────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────┘
```

## Prerequisites

### System Requirements

- **Linux kernel 5.8 or higher** with eBPF support enabled
- **Root privileges** or `CAP_BPF` + `CAP_NET_ADMIN` capabilities
- **LLVM/Clang** for compiling eBPF programs
- **Rust toolchain** (stable + nightly)

### Supported Distributions

- Ubuntu 20.04 LTS or newer
- Fedora 33 or newer
- Debian 11 or newer
- Arch Linux (latest)
- Other Linux distributions with kernel 5.8+

### Check Your Kernel Version

```bash
uname -r
# Should be >= 5.8.0
```

## Installation

### 1. Install System Dependencies

#### Ubuntu/Debian

```bash
sudo apt-get update
sudo apt-get install -y \
    build-essential \
    clang \
    llvm \
    libelf-dev \
    linux-headers-$(uname -r) \
    pkg-config \
    iproute2
```

#### Fedora/RHEL

```bash
sudo dnf install -y \
    clang \
    llvm \
    elfutils-libelf-devel \
    kernel-devel \
    iproute
```

#### Arch Linux

```bash
sudo pacman -S clang llvm libelf linux-headers
```

### 2. Install Rust Toolchain

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# Install nightly toolchain (pin to known working build)
rustup toolchain install nightly-2024-02-15 --component rust-src
export CARGO_UNSTABLE_EDITION2024=1
# Patch compiler-builtins to disable big-int intrinsics (not supported by BPF LLVM backend)
TOOLCHAIN_DIR="$HOME/.rustup/toolchains/nightly-2024-02-15-x86_64-unknown-linux-gnu"
perl -0pi -e 's/pub mod big;/#[cfg(not(target_arch = "bpf"))]\npub mod big;/' \
    "$TOOLCHAIN_DIR/lib/rustlib/src/rust/library/compiler-builtins/compiler-builtins/src/int/mod.rs"
perl -0pi -e 's/pub use big::{i256, u256};/#[cfg(not(target_arch = "bpf"))]\npub use big::{i256, u256};/' \
    "$TOOLCHAIN_DIR/lib/rustlib/src/rust/library/compiler-builtins/compiler-builtins/src/int/mod.rs"

# Install bpf-linker (required for eBPF compilation)
cargo install bpf-linker
```

### 3. Clone and Build the Project

```bash
# Clone the repository (or navigate to project directory)
cd ebpf-sniffer

# Build the eBPF kernel program first
cd ebpf-sniffer-ebpf
cargo +nightly-2024-02-15 build --release \
    -Z build-std=core \
    -Z build-std-features=compiler-builtins-mem,compiler-builtins-no-f16-f128 \
    --target bpfel-unknown-none
cd ..

# Build the userspace program
cd ebpf-sniffer
cargo build --release
cd ..
```

Or use the convenience script:

```bash
chmod +x run.sh
./run.sh
```

**Note:** Pinning to `nightly-2024-02-15` avoids recent regressions in the BPF backend. Use `-Z build-std=core` with the
listed build-std features so `core`/`compiler_builtins` are rebuilt from source for the `bpfel-unknown-none` target. The
patched `compiler-builtins` disables the `big` integer module when targeting BPF, which otherwise triggers LLVM errors
(`aggregate returns are not supported`).

### 4. Verify Build

```bash
# Check eBPF program
ls -lh target/bpfel-unknown-none/release/ebpf-sniffer-ebpf

# Check userspace program
ls -lh target/release/ebpf-sniffer
```

## Container Usage (Podman/Docker)

### ⚠️ Apple Silicon Limitations

eBPF development on Apple Silicon faces challenges:
- Podman VM is ARM64 (eBPF needs x86_64)
- Fedora CoreOS is immutable (can't install packages normally)

**Solutions below work around these limitations.**

### Practical Solutions for Apple Silicon Users

#### **Option 0: Lima VM with Rosetta (Best for Apple Silicon!)** ⭐

Lima can use macOS Rosetta 2 for better x86_64 emulation:

```bash
# Install Lima (if not already)
brew install lima

# Create and start eBPF VM (uses Rosetta)
limactl start --name=ebpf-dev lima-ebpf.yaml

# Build using helper script
./lima-build.sh

# Run in Lima VM
limactl shell ebpf-dev
cd $(pwd)  # Your project is auto-mounted
sudo ./target/release/ebpf-sniffer --iface eth0 --domains api.github.com --verbose
```

**Why Lima?** Uses Rosetta 2 instead of QEMU = more stable x86_64 emulation, fewer crashes.

#### **Option 1: Dev Container (Alternative)**

Use a persistent development container:

```bash
./dev-container.sh  # May hit QEMU issues
```

#### Option 1: Cloud Linux Instance (Recommended)

**AWS EC2:**
```bash
# Launch Amazon Linux 2023 t2.micro (free tier)
aws ec2 run-instances --image-id ami-xxx --instance-type t2.micro

# SSH in and build
ssh ec2-user@<instance-ip>
git clone <your-repo>
cd ebpf-sniffer
./run.sh
```

**DigitalOcean Droplet:**
```bash
# Create $6/month Ubuntu droplet
# SSH in and follow Linux build instructions
```

#### Option 2: Create x86_64 Podman Machine (May Hit QEMU Issues)

Create a separate x86_64 Podman machine:

```bash
# Create new x86_64 machine
podman machine init --cpus 4 --memory 4096 --disk-size 50 ebpf-x86

# Start it
podman machine start ebpf-x86

# Set as default
podman system connection default ebpf-x86

# Try dev container approach with x86_64 emulation
./dev-container.sh
```

**Warning:** May still hit rustc segfaults due to QEMU emulation.

#### Option 3: x86_64 Virtual Machine

Use **VirtualBox** or **UTM** with x86_64 Ubuntu:
```bash
# Install Ubuntu 22.04 x86_64 in VM
# Follow standard Linux build instructions
# Network interface will be available inside VM
```

#### Option 3: Use a Separate Linux Machine

If you have access to:
- Linux laptop/desktop
- Raspberry Pi with x86_64 OS
- Corporate Linux workstation

### Build Container Image (x86_64 Linux Only)

```bash
# Run from the ebpf-sniffer workspace root
podman build -t ebpf-sniffer -f Dockerfile .
```

This Dockerfile uses Fedora, installs the required LLVM/Clang + Rust nightly toolchains, and
builds both the eBPF object and userspace binary. Building directly on Apple Silicon is still
unsupported—use an x86_64 VM, Podman machine, or remote builder if you're on macOS ARM.

### Run in Container

**Important:** eBPF requires a Linux kernel. On macOS, Podman automatically uses its Linux VM.

```bash
# Run directly from macOS (Podman handles the VM transparently)
podman run --rm --privileged \
    --network host \
    --pid host \
    --cap-add SYS_ADMIN \
    --cap-add NET_ADMIN \
    --cap-add BPF \
    -v /sys/kernel/debug:/sys/kernel/debug:ro \
    ebpf-sniffer \
    --iface eth0 --domains api.github.com --verbose
```

**Why these flags?**
- `--privileged` - Full access to host devices (required for eBPF)
- `--network host` - Access VM's network interfaces
- `--pid host` - Access host process namespace
- `--cap-add` - Specific kernel capabilities for eBPF
- `-v /sys/kernel/debug` - Mount kernel debug filesystem

### Quick Start with Helper Script

```bash
# From macOS, just run the script
./run-container.sh eth0 api.github.com
```

The script automatically:
- Detects Podman/Docker
- Builds image if needed
- Runs with proper privileges

### Find VM Network Interface

To see which interface to use (inside the Podman VM):

```bash
podman machine ssh ip -br addr show
```

Common interfaces: `eth0`, `enp0s1`, `enp0s2`

## Usage

### Basic Usage

```bash
# Find your network interface
ip link show

# Capture HTTPS traffic to api.github.com
sudo ./target/release/ebpf-sniffer \
    --iface eth0 \
    --domains api.github.com
```

### Monitor Multiple Domains

```bash
sudo ./target/release/ebpf-sniffer \
    --iface wlan0 \
    --domains "api.github.com,example.com,google.com"
```

### Enable Verbose Logging

```bash
sudo ./target/release/ebpf-sniffer \
    --iface eth0 \
    --domains api.github.com \
    --verbose
```

### Export to CSV File

```bash
sudo ./target/release/ebpf-sniffer \
    --iface eth0 \
    --domains api.github.com \
    --output captured_packets.csv
```

### Full Example with All Options

```bash
sudo ./target/release/ebpf-sniffer \
    --iface eth0 \
    --domains "api.github.com,google.com,cloudflare.com" \
    --output packets.csv \
    --verbose
```

### Command-Line Options

```
Options:
  -i, --iface <IFACE>      Network interface to attach to (e.g., eth0, wlan0)
  -d, --domains <DOMAINS>  Comma-separated list of domains to monitor
  -o, --output <OUTPUT>    Optional output file for captured packets (CSV format)
  -v, --verbose            Enable verbose logging
  -h, --help              Print help
  -V, --version           Print version
```

## Output Format

### Console Output

```
[INFO] Starting eBPF HTTPS Traffic Sniffer
[INFO] Resolving domains...
[INFO]   api.github.com -> [140.82.121.6]
[INFO] Loaded 1 target IPs into eBPF map
[INFO] Attached to eth0 egress
[INFO] Processing events on 8 CPUs
[INFO] Captured packet: 192.168.1.100:54321 -> 140.82.121.6:443 (517 bytes)
[INFO]   → TLS handshake detected
[INFO]   → SNI: api.github.com
```

### CSV Output Format

When using `--output`, packets are saved to a CSV file:

```csv
timestamp,src_ip,src_port,dst_ip,dst_port,data_len,payload_hex
1234567890123456,192.168.1.100,54321,140.82.121.6,443,517,1603010...
```

## What Gets Captured

The sniffer captures:

- **Source and destination IPs and ports**
- **Packet timestamps** (kernel time in nanoseconds)
- **Full TCP payload** (up to 1500 bytes)
- **TLS handshake data** including Client Hello
- **SNI (Server Name Indication)** from TLS handshakes
- **HTTP/2 connection preface** and gRPC frames

## Protocol Detection

### TLS/SSL Detection

Automatically detects TLS handshakes by looking for:
- Content Type: `0x16` (Handshake)
- Version: `0x03 0x01` (TLS 1.0), `0x03 0x03` (TLS 1.2), etc.

### SNI Extraction

Parses TLS Client Hello messages to extract the Server Name Indication (SNI), which reveals the target domain even though the connection is encrypted.

### HTTP/2 and gRPC Detection

Recognizes:
- HTTP/2 connection preface: `PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n`
- HTTP/2 frame headers
- gRPC protocol buffers over HTTP/2

## Security Considerations

### What This Tool Does

- **Monitors traffic** at the kernel level
- **Captures encrypted data** (but cannot decrypt it)
- **Extracts metadata** like SNI, IPs, ports
- **Does NOT decrypt** HTTPS/TLS traffic
- **Does NOT modify** any packets (pass-through only)

### Ethical Use

This tool is designed for:
- **Network debugging and troubleshooting**
- **Security research and education**
- **Monitoring your own systems**
- **Authorized security testing**

**DO NOT use this tool to:**
- Monitor traffic on systems you don't own or have permission to monitor
- Intercept or decrypt other people's communications
- Violate privacy laws or regulations (GDPR, CCPA, etc.)

### Required Permissions

The tool requires elevated privileges to:
- Load eBPF programs into the kernel
- Attach to network interfaces
- Access kernel maps and perf buffers

Use `sudo` or grant specific capabilities:

```bash
# Option 1: Run with sudo
sudo ./target/release/ebpf-sniffer ...

# Option 2: Grant specific capabilities
sudo setcap cap_bpf,cap_net_admin+eip ./target/release/ebpf-sniffer
./target/release/ebpf-sniffer ...
```

## Troubleshooting

### Error: "Operation not permitted"

**Cause:** Insufficient permissions to load eBPF programs.

**Solution:**
```bash
# Run with sudo
sudo ./target/release/ebpf-sniffer ...

# Or grant capabilities
sudo setcap cap_bpf,cap_net_admin+eip ./target/release/ebpf-sniffer
```

### Error: "Failed to increase RLIMIT_MEMLOCK"

**Cause:** Memory limit too low for eBPF maps.

**Solution:**
```bash
# Temporarily increase limit
ulimit -l unlimited

# Permanently increase (add to /etc/security/limits.conf)
* soft memlock unlimited
* hard memlock unlimited
```

### Error: "No such device" when attaching

**Cause:** Network interface name is incorrect or doesn't exist.

**Solution:**
```bash
# List available interfaces
ip link show

# Use the correct interface name
sudo ./target/release/ebpf-sniffer --iface <correct-name> ...
```

### Error: "DNS lookup failed"

**Cause:** Domain name cannot be resolved to an IP address.

**Solution:**
- Check your internet connection
- Verify domain name spelling
- Try using `dig` or `nslookup` to test DNS resolution:
  ```bash
  dig api.github.com
  ```

### No Packets Captured

**Possible causes:**
1. **No traffic to monitored domains** - Try generating traffic:
   ```bash
   curl https://api.github.com
   ```

2. **Wrong interface** - Make sure you're using the interface that handles internet traffic:
   ```bash
   ip route show default
   # Use the interface listed (e.g., eth0, wlan0)
   ```

3. **Firewall or routing** - Packets might be taking a different path

### Kernel Verifier Errors

**Cause:** eBPF program failed kernel verification (safety checks).

**Solution:**
- Ensure kernel is 5.8 or newer
- Rebuild with latest dependencies
- Check kernel logs: `sudo dmesg | tail -50`

### Build Errors

**Missing `bpfel-unknown-none` target:**
```bash
rustup target add bpfel-unknown-none --toolchain nightly
```

**LLVM/Clang not found:**
```bash
# Ubuntu/Debian
sudo apt-get install clang llvm

# Fedora
sudo dnf install clang llvm
```

**Kernel headers missing:**
```bash
# Ubuntu/Debian
sudo apt-get install linux-headers-$(uname -r)

# Fedora
sudo dnf install kernel-devel
```

## Performance

- **Minimal overhead**: eBPF runs in kernel space with optimized bytecode
- **Zero packet loss**: All packets are passed through (TC_ACT_OK)
- **Multi-CPU scaling**: Events processed in parallel across all CPU cores
- **Efficient filtering**: Only monitors specified IPs on port 443

## Limitations

- **IPv4 only**: Currently does not support IPv6 (can be added)
- **TCP only**: Only monitors TCP traffic (no UDP)
- **Port 443 only**: Hardcoded to HTTPS (can be made configurable)
- **1500 byte payload limit**: Captures first 1500 bytes per packet
- **No decryption**: Cannot decrypt TLS/HTTPS traffic (by design)

## Development

### Project Structure

```
ebpf-sniffer/
├── Cargo.toml                      # Workspace configuration
├── .cargo/
│   └── config.toml                 # Cargo build settings
├── ebpf-sniffer/                   # Userspace program
│   ├── Cargo.toml
│   ├── build.rs                    # Build script
│   └── src/
│       └── main.rs                 # Userspace loader and processor
└── ebpf-sniffer-ebpf/              # Kernel eBPF program
    ├── Cargo.toml
    └── src/
        └── main.rs                 # TC egress classifier

```

### Adding Features

**Support IPv6:**
- Add IPv6 header parsing in kernel code
- Update `PacketInfo` struct to support IPv6 addresses
- Modify DNS resolution to include IPv6

**Configurable ports:**
- Add port parameter to CLI args
- Pass port list to eBPF via map
- Update filtering logic in kernel code

**Better TLS parsing:**
- Implement full TLS parser
- Extract more handshake details
- Identify TLS version and cipher suites

## Testing

### Generate Test Traffic

```bash
# Terminal 1: Start the sniffer
sudo ./target/release/ebpf-sniffer \
    --iface eth0 \
    --domains api.github.com \
    --verbose

# Terminal 2: Generate HTTPS requests
curl https://api.github.com
curl https://api.github.com/users/octocat
```

### Expected Output

You should see packet captures with:
- TLS handshake detection
- SNI extraction showing "api.github.com"
- Packet metadata (IPs, ports, timestamps)

## Additional Resources

- [eBPF Documentation](https://ebpf.io/what-is-ebpf/)
- [Aya - Rust eBPF Framework](https://aya-rs.dev/)
- [TC Traffic Control](https://man7.org/linux/man-pages/man8/tc.8.html)
- [TLS 1.3 RFC](https://www.rfc-editor.org/rfc/rfc8446)
- [Linux Kernel BPF](https://www.kernel.org/doc/html/latest/bpf/)

## License

This project is provided for educational and research purposes. Use responsibly and ethically.

## Contributing

Contributions are welcome! Areas for improvement:
- IPv6 support
- UDP/QUIC support
- Configurable port filtering
- Better protocol analysis
- Performance optimizations
- Additional export formats (PCAP, JSON)

## Authors

Created as a demonstration of Rust + eBPF for network monitoring and security research.

use anyhow::{Context, Result, anyhow};
use aya::{
    include_bytes_aligned,
    maps::{HashMap, perf::AsyncPerfEventArray},
    programs::{tc, SchedClassifier, TcAttachType},
    util::online_cpus,
    Bpf,
};
use aya_log::BpfLogger;
use bytes::BytesMut;
use clap::Parser;
use dns_lookup::lookup_host;
use log::{debug, info, warn, error};
use std::{
    fs::OpenOptions,
    io::Write,
    net::IpAddr,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::{signal, task};

/// Maximum packet payload size (must match kernel code)
const MAX_PAYLOAD_SIZE: usize = 1500;

/// Packet metadata information (must match kernel struct)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct PacketInfo {
    src_ip: u32,
    dst_ip: u32,
    src_port: u16,
    dst_port: u16,
    data_len: u32,
    timestamp: u64,
}

/// Complete packet data structure (must match kernel struct)
#[repr(C)]
struct PacketData {
    info: PacketInfo,
    data: [u8; MAX_PAYLOAD_SIZE],
}

/// Command-line arguments
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Network interface to attach to (e.g., eth0, wlan0)
    #[arg(short, long)]
    iface: String,

    /// Comma-separated list of domains to monitor (e.g., api.github.com,example.com)
    #[arg(short, long)]
    domains: String,

    /// Optional output file for captured packets (CSV format)
    #[arg(short, long)]
    output: Option<String>,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Setup logging
    if args.verbose {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug"))
            .init();
    } else {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
            .init();
    }

    info!("Starting eBPF HTTPS Traffic Sniffer");

    // Bump the memlock rlimit to allow eBPF maps
    let rlim = libc::rlimit {
        rlim_cur: libc::RLIM_INFINITY,
        rlim_max: libc::RLIM_INFINITY,
    };
    let ret = unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &rlim) };
    if ret != 0 {
        warn!("Failed to increase RLIMIT_MEMLOCK, may encounter issues loading eBPF maps");
    }

    // Load compiled eBPF program
    #[cfg(debug_assertions)]
    let mut bpf = Bpf::load(include_bytes_aligned!(
        "../../target/bpfel-unknown-none/debug/ebpf-sniffer-ebpf"
    ))?;
    #[cfg(not(debug_assertions))]
    let mut bpf = Bpf::load(include_bytes_aligned!(
        "../../target/bpfel-unknown-none/release/ebpf-sniffer-ebpf"
    ))?;

    // Optionally attach eBPF logger
    if let Err(e) = BpfLogger::init(&mut bpf) {
        warn!("Failed to initialize eBPF logger: {}", e);
    }

    // Parse and resolve domains to IP addresses
    let domains: Vec<&str> = args.domains.split(',').map(|s| s.trim()).collect();
    let mut target_ips = Vec::new();

    info!("Resolving domains...");
    for domain in &domains {
        match resolve_domain(domain) {
            Ok(ips) => {
                info!("  {} -> {:?}", domain, ips);
                target_ips.extend(ips);
            }
            Err(e) => {
                warn!("  Failed to resolve {}: {}", domain, e);
            }
        }
    }

    if target_ips.is_empty() {
        return Err(anyhow!("No target IPs resolved from provided domains"));
    }

    // Populate TARGET_IPS map in eBPF program
    let mut target_ips_map: HashMap<_, u32, u8> = HashMap::try_from(bpf.map_mut("TARGET_IPS")?)?;

    for ip in &target_ips {
        if let IpAddr::V4(ipv4) = ip {
            let ip_u32 = u32::from(*ipv4);
            target_ips_map
                .insert(ip_u32, 1, 0)
                .context("Failed to insert IP into TARGET_IPS map")?;
            debug!("Added target IP to eBPF map: {}", ipv4);
        }
    }

    info!("Loaded {} target IPs into eBPF map", target_ips.len());

    // Load and attach TC program
    let program: &mut SchedClassifier = bpf
        .program_mut("ebpf_sniffer")
        .context("Failed to find ebpf_sniffer program")?
        .try_into()?;
    program.load()?;

    // Attach to the network interface egress
    program.attach(&args.iface, TcAttachType::Egress)?;
    info!("Attached to {} egress", args.iface);

    // Setup perf event array for receiving packets
    let mut perf_array = AsyncPerfEventArray::try_from(bpf.take_map("PACKET_EVENTS")?)?;

    // Atomic flag for graceful shutdown
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = shutdown.clone();

    // Spawn Ctrl-C handler
    task::spawn(async move {
        signal::ctrl_c().await.expect("Failed to listen for Ctrl-C");
        info!("Received Ctrl-C, shutting down...");
        shutdown_clone.store(true, Ordering::Relaxed);
    });

    // Spawn tasks for each CPU to process perf events
    let cpus = online_cpus().context("Failed to get online CPUs")?;
    info!("Processing events on {} CPUs", cpus.len());

    let output_file = args.output.clone();

    for cpu_id in cpus {
        let mut buf = perf_array.open(cpu_id, None)?;
        let shutdown = shutdown.clone();
        let output_file = output_file.clone();

        task::spawn(async move {
            let mut buffers = (0..10)
                .map(|_| BytesMut::with_capacity(4096))
                .collect::<Vec<_>>();

            loop {
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }

                let events = match buf.read_events(&mut buffers).await {
                    Ok(events) => events,
                    Err(e) => {
                        error!("Error reading perf events on CPU {}: {}", cpu_id, e);
                        continue;
                    }
                };

                for buf in buffers.iter_mut().take(events.read) {
                    if let Err(e) = handle_packet(buf, output_file.as_deref()) {
                        warn!("Error handling packet: {}", e);
                    }
                }
            }

            info!("CPU {} event processor shutting down", cpu_id);
        });
    }

    // Wait for shutdown signal
    while !shutdown.load(Ordering::Relaxed) {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    info!("Detaching eBPF program...");
    Ok(())
}

/// Resolve a domain name to IPv4 addresses
fn resolve_domain(domain: &str) -> Result<Vec<IpAddr>> {
    let ips: Vec<IpAddr> = lookup_host(domain)
        .with_context(|| format!("DNS lookup failed for {}", domain))?
        .into_iter()
        .filter(|ip| ip.is_ipv4()) // Only IPv4 for now
        .collect();

    if ips.is_empty() {
        return Err(anyhow!("No IPv4 addresses found for {}", domain));
    }

    Ok(ips)
}

/// Handle a captured packet
fn handle_packet(buf: &BytesMut, output_file: Option<&str>) -> Result<()> {
    // Safety: We need to interpret the buffer as a PacketData struct
    // This requires that the buffer is properly aligned and sized
    if buf.len() < std::mem::size_of::<PacketData>() {
        return Err(anyhow!("Buffer too small for PacketData"));
    }

    let packet_ptr = buf.as_ptr() as *const PacketData;
    let packet = unsafe { packet_ptr.read_unaligned() };

    // Convert IPs to human-readable format
    let src_ip = std::net::Ipv4Addr::from(packet.info.src_ip);
    let dst_ip = std::net::Ipv4Addr::from(packet.info.dst_ip);

    // Get actual payload length
    let payload_len = packet.info.data_len.min(MAX_PAYLOAD_SIZE as u32) as usize;
    let payload = &packet.data[..payload_len];

    // Log packet information
    info!(
        "Captured packet: {}:{} -> {}:{} ({} bytes)",
        src_ip, packet.info.src_port, dst_ip, packet.info.dst_port, payload_len
    );

    // Analyze packet content
    analyze_packet_content(payload);

    // Optionally write to file
    if let Some(path) = output_file {
        write_packet_to_file(path, &packet)?;
    }

    Ok(())
}

/// Analyze packet content to identify protocols and extract information
fn analyze_packet_content(data: &[u8]) {
    if data.is_empty() {
        return;
    }

    // Check for TLS handshake
    if is_tls_handshake(data) {
        info!("  → TLS handshake detected");

        // Try to extract SNI
        if let Some(sni) = extract_sni(data) {
            info!("  → SNI: {}", sni);
        }
    }

    // Check for HTTP/2 or gRPC
    if is_http2_or_grpc(data) {
        info!("  → HTTP/2 or gRPC traffic detected");
    }

    // Display first few bytes as hex for debugging
    let preview_len = data.len().min(32);
    debug!("  → Payload preview: {}", hex::encode(&data[..preview_len]));
}

/// Check if data contains a TLS handshake
fn is_tls_handshake(data: &[u8]) -> bool {
    // TLS handshake starts with: 0x16 (Handshake), 0x03 (SSL 3.x / TLS 1.x)
    data.len() >= 3 && data[0] == 0x16 && data[1] == 0x03
}

/// Check if data is HTTP/2 or gRPC
fn is_http2_or_grpc(data: &[u8]) -> bool {
    // HTTP/2 connection preface: "PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n"
    if data.len() >= 24 {
        return &data[..3] == b"PRI";
    }

    // gRPC uses HTTP/2 framing - check for frame header pattern
    // Format: [length:3][type:1][flags:1][stream_id:4]
    if data.len() >= 9 {
        // Check if it looks like HTTP/2 frame (heuristic)
        let frame_type = data[3];
        // Common frame types: DATA=0, HEADERS=1, SETTINGS=4, PING=6
        return frame_type <= 9;
    }

    false
}

/// Extract SNI (Server Name Indication) from TLS ClientHello
fn extract_sni(data: &[u8]) -> Option<String> {
    // This is a simplified SNI extraction
    // Full implementation would need proper TLS parser
    if data.len() < 43 {
        return None;
    }

    // Skip: Content Type (1), Version (2), Length (2), Handshake Type (1),
    //       Length (3), Version (2), Random (32) = 43 bytes

    let mut offset = 43;

    // Session ID Length
    if offset >= data.len() {
        return None;
    }
    let session_id_len = data[offset] as usize;
    offset += 1 + session_id_len;

    // Cipher Suites Length
    if offset + 2 > data.len() {
        return None;
    }
    let cipher_suites_len = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
    offset += 2 + cipher_suites_len;

    // Compression Methods Length
    if offset >= data.len() {
        return None;
    }
    let compression_len = data[offset] as usize;
    offset += 1 + compression_len;

    // Extensions Length
    if offset + 2 > data.len() {
        return None;
    }
    let extensions_len = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
    offset += 2;

    let extensions_end = offset + extensions_len;
    if extensions_end > data.len() {
        return None;
    }

    // Parse extensions
    while offset + 4 <= extensions_end {
        let ext_type = u16::from_be_bytes([data[offset], data[offset + 1]]);
        let ext_len = u16::from_be_bytes([data[offset + 2], data[offset + 3]]) as usize;
        offset += 4;

        // SNI extension type is 0
        if ext_type == 0 && offset + ext_len <= data.len() {
            // SNI format: [list_len:2][type:1][name_len:2][name:name_len]
            if ext_len >= 5 {
                let name_offset = offset + 5;
                let name_len = u16::from_be_bytes([data[offset + 3], data[offset + 4]]) as usize;

                if name_offset + name_len <= data.len() {
                    if let Ok(sni) = std::str::from_utf8(&data[name_offset..name_offset + name_len])
                    {
                        return Some(sni.to_string());
                    }
                }
            }
        }

        offset += ext_len;
    }

    None
}

/// Write packet data to CSV file
fn write_packet_to_file(path: &str, packet: &PacketData) -> Result<()> {
    let file_exists = Path::new(path).exists();

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .context("Failed to open output file")?;

    // Write CSV header if file is new
    if !file_exists {
        writeln!(
            file,
            "timestamp,src_ip,src_port,dst_ip,dst_port,data_len,payload_hex"
        )?;
    }

    // Convert IPs to readable format
    let src_ip = std::net::Ipv4Addr::from(packet.info.src_ip);
    let dst_ip = std::net::Ipv4Addr::from(packet.info.dst_ip);

    // Get actual payload
    let payload_len = packet.info.data_len.min(MAX_PAYLOAD_SIZE as u32) as usize;
    let payload_hex = hex::encode(&packet.data[..payload_len]);

    // Write CSV row
    writeln!(
        file,
        "{},{},{},{},{},{},{}",
        packet.info.timestamp,
        src_ip,
        packet.info.src_port,
        dst_ip,
        packet.info.dst_port,
        packet.info.data_len,
        payload_hex
    )?;

    Ok(())
}

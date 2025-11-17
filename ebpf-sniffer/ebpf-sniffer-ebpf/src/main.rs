#![no_std]
#![no_main]

use aya_ebpf::{
    bindings::TC_ACT_OK,
    macros::{classifier, map},
    maps::{HashMap, PerfEventArray},
    programs::TcContext,
};
use core::mem;
use network_types::{
    eth::{EthHdr, EtherType},
    ip::{Ipv4Hdr, IpProto},
    tcp::TcpHdr,
};

/// Maximum packet payload size to capture
const MAX_PAYLOAD_SIZE: usize = 1500;

/// Packet metadata information
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PacketInfo {
    pub src_ip: u32,
    pub dst_ip: u32,
    pub src_port: u16,
    pub dst_port: u16,
    pub data_len: u32,
    pub timestamp: u64,
}

/// Complete packet data structure sent to userspace
#[repr(C)]
pub struct PacketData {
    pub info: PacketInfo,
    pub data: [u8; MAX_PAYLOAD_SIZE],
}

/// Map storing target IPs to monitor (key=IP, value=1 if enabled)
#[map]
static TARGET_IPS: HashMap<u32, u8> = HashMap::with_max_entries(1024, 0);

/// PerfEventArray for sending captured packets to userspace
#[map]
static PACKET_EVENTS: PerfEventArray<PacketData> = PerfEventArray::new(0);

/// TC egress classifier that captures HTTPS traffic to target domains
#[classifier]
pub fn ebpf_sniffer(ctx: TcContext) -> i32 {
    match try_ebpf_sniffer(ctx) {
        Ok(ret) => ret,
        Err(_) => TC_ACT_OK, // Always allow packet through on error
    }
}

fn try_ebpf_sniffer(ctx: TcContext) -> Result<i32, ()> {
    // Parse Ethernet header
    let eth_hdr: *const EthHdr = unsafe { ptr_at(&ctx, 0)? };

    // Check if this is an IPv4 packet
    match unsafe { (*eth_hdr).ether_type } {
        EtherType::Ipv4 => {}
        _ => return Ok(TC_ACT_OK), // Not IPv4, pass through
    }

    // Parse IPv4 header
    let ipv4_hdr: *const Ipv4Hdr = unsafe { ptr_at(&ctx, EthHdr::LEN)? };

    // Check if this is TCP
    let ip_proto = unsafe { (*ipv4_hdr).proto };
    if ip_proto != IpProto::Tcp {
        return Ok(TC_ACT_OK); // Not TCP, pass through
    }

    // Get destination IP (in network byte order)
    let dst_ip = unsafe { u32::from_be((*ipv4_hdr).dst_addr) };

    // Check if destination IP is in our target list
    if unsafe { TARGET_IPS.get(&dst_ip) }.is_none() {
        return Ok(TC_ACT_OK); // Not a target IP, pass through
    }

    // Parse TCP header
    let tcp_hdr: *const TcpHdr = unsafe { ptr_at(&ctx, EthHdr::LEN + Ipv4Hdr::LEN)? };

    // Get destination port (convert from network to host byte order)
    let dst_port = unsafe { u16::from_be((*tcp_hdr).dest) };

    // Only capture HTTPS traffic (port 443)
    if dst_port != 443 {
        return Ok(TC_ACT_OK); // Not HTTPS, pass through
    }

    // Get source IP and port for logging
    let src_ip = unsafe { u32::from_be((*ipv4_hdr).src_addr) };
    let src_port = unsafe { u16::from_be((*tcp_hdr).source) };

    // Calculate TCP header length (data offset * 4)
    let tcp_hdr_len = unsafe { ((*tcp_hdr).doff() as usize) * 4 };

    // Calculate payload offset
    let payload_offset = EthHdr::LEN + Ipv4Hdr::LEN + tcp_hdr_len;

    // Get total packet length
    let packet_len = ctx.data_end() - ctx.data();

    // Calculate payload length
    let payload_len = if packet_len > payload_offset {
        packet_len - payload_offset
    } else {
        0
    };

    // Only capture packets with payload
    if payload_len == 0 {
        return Ok(TC_ACT_OK);
    }

    // Prepare packet data structure
    let mut packet = PacketData {
        info: PacketInfo {
            src_ip,
            dst_ip,
            src_port,
            dst_port,
            data_len: payload_len as u32,
            timestamp: unsafe { aya_ebpf::helpers::bpf_ktime_get_ns() },
        },
        data: [0u8; MAX_PAYLOAD_SIZE],
    };

    // Copy payload data (bounded by MAX_PAYLOAD_SIZE)
    let copy_len = if payload_len > MAX_PAYLOAD_SIZE {
        MAX_PAYLOAD_SIZE
    } else {
        payload_len
    };

    // Safely copy payload data with bounds checking (required by eBPF verifier)
    for i in 0..copy_len {
        // Verifier requires explicit bounds check
        if i >= MAX_PAYLOAD_SIZE {
            break;
        }

        // Read byte from packet
        if let Ok(byte) = read_byte_at(&ctx, payload_offset + i) {
            packet.data[i] = byte;
        } else {
            break; // Stop on read error
        }
    }

    // Send packet data to userspace
    PACKET_EVENTS.output(&ctx, &packet, 0);

    Ok(TC_ACT_OK) // Always allow packet through
}

/// Safely get a pointer to data at a given offset with bounds checking
#[inline(always)]
unsafe fn ptr_at<T>(ctx: &TcContext, offset: usize) -> Result<*const T, ()> {
    let start = ctx.data();
    let end = ctx.data_end();
    let len = mem::size_of::<T>();

    if start + offset + len > end {
        return Err(());
    }

    Ok((start + offset) as *const T)
}

/// Safely read a single byte at offset with bounds checking
#[inline(always)]
fn read_byte_at(ctx: &TcContext, offset: usize) -> Result<u8, ()> {
    let start = ctx.data();
    let end = ctx.data_end();

    if start + offset + 1 > end {
        return Err(());
    }

    unsafe { Ok(*((start + offset) as *const u8)) }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe { core::hint::unreachable_unchecked() }
}

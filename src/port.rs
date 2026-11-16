//! 端口探测：判断服务是否就绪、是否真的已经停止。
//!
//! 数据来源是 `GetExtendedTcpTable(TCP_TABLE_OWNER_PID_LISTENER)` 一次查询，
//! 监听端口和占用它的进程 ID 一起拿到。
//!
//! 与 C# 版的两点不同，都是净简化：
//!   - C# 版用 `GetActiveTcpListeners()` 判断监听，另外再 `netstat -ano` 抓一遍文本、
//!     按列拆字符串找 pid。这里是同一次系统调用的同一份数据。
//!   - 不再需要解析任何文本，也就不存在“列数不够就跳过”这类兜底分支。

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{BOOL, ERROR_INSUFFICIENT_BUFFER, NO_ERROR};
use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCPROW_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
};
use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6};

/// 一次监听端口查询的结果：端口与持有它的进程 ID。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Listener {
    /// 监听端口。
    pub port: u16,

    /// 持有这个监听套接字的进程 ID。
    pub process_id: u32,
}

/// 端口上是否有服务在监听。
pub fn is_listening(port: u16) -> bool {
    listeners().iter().any(|listener| listener.port == port)
}

/// 占用指定端口的进程 ID（可能不止一个：IPv4 与 IPv6 各一个，或者 SO_REUSEADDR）。
pub fn owning_process_ids(port: u16) -> Vec<u32> {
    let ids: BTreeSet<u32> = listeners()
        .into_iter()
        .filter(|listener| listener.port == port && listener.process_id > 0)
        .map(|listener| listener.process_id)
        .collect();

    ids.into_iter().collect()
}

/// 等待端口进入指定状态。
///
/// 关掉服务后端口回收可能有延迟（实测可达数秒），所以停止流程必须用带超时的轮询，
/// 而不能杀完就认为已停止。
pub fn wait_for_state(port: u16, should_be_listening: bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;

    loop {
        if is_listening(port) == should_be_listening {
            return true;
        }

        if Instant::now() >= deadline {
            return is_listening(port) == should_be_listening;
        }

        std::thread::sleep(Duration::from_millis(100));
    }
}

/// 所有监听端口。查询失败时返回空列表，让调用方走等待／重试。
fn listeners() -> Vec<Listener> {
    let mut all = listeners_for_family(AF_INET.0 as u32, parse_ipv4_table);
    all.extend(listeners_for_family(AF_INET6.0 as u32, parse_ipv6_table));
    all
}

/// 查一个地址族，交给 `parse` 解析表里的行。
fn listeners_for_family(
    address_family: u32,
    parse: fn(&[u8]) -> Vec<Listener>,
) -> Vec<Listener> {
    /*
     * 先问大小再查一遍：条目数随时会变，固定缓冲区只能是猜。
     * 第二次查询若因为条目变多又不够，就再来一轮。
     */
    for _ in 0..4 {
        let mut size: u32 = 0;

        // SAFETY: 只传大小指针，缓冲区为空，函数只写这一处
        let status = unsafe {
            GetExtendedTcpTable(
                None,
                &mut size,
                BOOL(0),
                address_family,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };

        if status != ERROR_INSUFFICIENT_BUFFER.0 && status != NO_ERROR.0 {
            return Vec::new();
        }

        if size == 0 {
            return Vec::new();
        }

        let mut buffer = vec![0u8; size as usize];

        // SAFETY: buffer 的长度就是刚问出来的 size，函数至多写这么多
        let status = unsafe {
            GetExtendedTcpTable(
                Some(buffer.as_mut_ptr().cast()),
                &mut size,
                BOOL(0),
                address_family,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };

        if status == ERROR_INSUFFICIENT_BUFFER.0 {
            // 条目在我们准备缓冲区的这一下变多了，重来
            continue;
        }

        if status != NO_ERROR.0 {
            return Vec::new();
        }

        buffer.truncate(size as usize);
        return parse(&buffer);
    }

    Vec::new()
}

/// MIB_TCPTABLE_OWNER_PID：开头一个 dwNumEntries，后面跟一串 MIB_TCPROW_OWNER_PID。
fn parse_ipv4_table(buffer: &[u8]) -> Vec<Listener> {
    let (count, body) = split_table(buffer);
    let row_size = std::mem::size_of::<MIB_TCPROW_OWNER_PID>();

    (0..count)
        .filter_map(|index| {
            let offset = index * row_size;
            let bytes = body.get(offset..offset + row_size)?;

            // SAFETY: bytes 的长度等于结构体大小，对齐由 vec![0u8] 的整数分配保证不足，
            // 所以用 read_unaligned 而不是直接取引用
            let row: MIB_TCPROW_OWNER_PID =
                unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast()) };

            Some(Listener {
                port: network_order_port(row.dwLocalPort),
                process_id: row.dwOwningPid,
            })
        })
        .collect()
}

/// MIB_TCP6TABLE_OWNER_PID：同样的结构，行是 MIB_TCP6ROW_OWNER_PID。
fn parse_ipv6_table(buffer: &[u8]) -> Vec<Listener> {
    let (count, body) = split_table(buffer);
    let row_size = std::mem::size_of::<MIB_TCP6ROW_OWNER_PID>();

    (0..count)
        .filter_map(|index| {
            let offset = index * row_size;
            let bytes = body.get(offset..offset + row_size)?;

            // SAFETY: 同 parse_ipv4_table
            let row: MIB_TCP6ROW_OWNER_PID =
                unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast()) };

            Some(Listener {
                port: network_order_port(row.dwLocalPort),
                process_id: row.dwOwningPid,
            })
        })
        .collect()
}

/// 表头：前 4 字节是条目数，之后是行。
fn split_table(buffer: &[u8]) -> (usize, &[u8]) {
    if buffer.len() < 4 {
        return (0, &[]);
    }

    let count = u32::from_ne_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]) as usize;

    (count, &buffer[4..])
}

/// 表里的端口是网络字节序，而且塞在一个 DWORD 的低 16 位里：
/// 低地址那字节是高字节，所以要倒着拼。
fn network_order_port(raw: u32) -> u16 {
    let bytes = raw.to_le_bytes();

    u16::from_be_bytes([bytes[0], bytes[1]])
}

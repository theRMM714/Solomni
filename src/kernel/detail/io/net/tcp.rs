//! 目的：回环 TCP 的三个原语——预订一个空闲端口、拨回环、等对端把端口交出来。
//! 管：127.0.0.1 上的 bind / connect / 带预算的重试等待；超时与「对端已死」的措辞由调用方给（`what` / `gone`）；只碰回环，别在这里开第二套地址策略。
//! 不管：端口后面是谁、连上之后说什么话（通道的 JSON、中继的帧都是业务）；出站去别的机器归 `outbound` 的传输栈。
//! 联动：虚拟机的通道等待与端口预订、内核中继的拨号都经 `src/kernel/api/net.rs` 取用。

use std::io;
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

/// 目的：一块已经占住的回环端口——拿 `port()` 报给要 bind 的子进程，`release` 或离开作用域就放开。
/// 约束：只是把「挑端口」与「子进程 bind」之间的碰撞面压小，不是保留——放掉之后别人也能抢。
pub struct ReservedPort {
    listener: TcpListener,
    port: u16,
}

impl ReservedPort {
    /// 目的：系统分配的实际端口。
    pub fn port(&self) -> u16 {
        self.port
    }

    /// 目的：提前放开（子进程马上要自己 bind 时用）。
    pub fn release(self) {
        drop(self.listener);
    }
}

/// 目的：占一个系统分配的空闲回环端口，别立刻放——一直占到 `release`。
pub fn reserve_port() -> Result<ReservedPort, String> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .map_err(|e| format!("占不到回环端口：{}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("占不到回环端口：{}", e))?
        .port();
    Ok(ReservedPort { listener, port })
}

/// 目的：拨本机回环端口（唯一一处写死回环客户端地址的地方）。
pub fn dial(port: u16) -> io::Result<TcpStream> {
    TcpStream::connect((Ipv4Addr::LOCALHOST, port))
}

/// 目的：等回环端口可连：每 50ms 试一次，超过预算报超时；对端先死由 `gone` 说明。
/// 参数：`what` 写进超时文案（如「QGA 通道」）；`gone` 返回 `Some(为什么)` 表示对端已经不在了。
pub fn await_loopback(
    port: u16,
    what: &str,
    budget: Duration,
    gone: &mut dyn FnMut() -> Option<String>,
) -> Result<TcpStream, String> {
    let deadline = Instant::now() + budget;
    loop {
        match dial(port) {
            Ok(stream) => return Ok(stream),
            Err(e) => {
                if let Some(why) = gone() {
                    return Err(format!("{}：{}", why, e));
                }
                if Instant::now() >= deadline {
                    return Err(format!("等{}超时（端口 {}）：{}", what, port, e));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reserved_port_is_free_again_after_release() {
        let reserved = reserve_port().expect("占端口");
        let port = reserved.port();
        assert!(port > 0);
        reserved.release();
        // 放开之后同一个端口能重新 bind（系统可能给别的端口，那就再占一次也算通）。
        let again = TcpListener::bind((Ipv4Addr::LOCALHOST, port));
        assert!(again.is_ok(), "放开后端口应能重新绑定：{:?}", again.err());
    }

    #[test]
    fn awaiting_a_port_that_nobody_serves_times_out_with_the_caller_word() {
        let reserved = reserve_port().expect("占端口");
        let port = reserved.port();
        reserved.release();
        let mut alive = || None;
        let started = Instant::now();
        let out = await_loopback(port, "某通道", Duration::from_millis(120), &mut alive);
        let text = out.expect_err("没人监听就该超时");
        assert!(text.contains("等某通道超时"), "{}", text);
        assert!(
            started.elapsed() >= Duration::from_millis(100),
            "预算要真的等"
        );
    }

    #[test]
    fn a_dead_peer_is_reported_before_the_budget_runs_out() {
        let reserved = reserve_port().expect("占端口");
        let port = reserved.port();
        reserved.release();
        let mut dead = || Some("对端已退出（exit status: 1）".to_string());
        let out = await_loopback(port, "某通道", Duration::from_secs(30), &mut dead);
        assert!(
            out.unwrap_err().contains("对端已退出"),
            "对端的死因要原样带出"
        );
    }
}

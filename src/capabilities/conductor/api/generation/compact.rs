//! 目的：手动压缩：让模型自己压，队列只占「取 / 交」两步。
//! 管：压不动如实说，不静默降级。
//! 不管：自动压缩（在会话收尾里）、命令分发。
//! 联动：`api/handle.rs`、`capabilities/collab`

use super::super::*;

impl ConductorHandle {
    /// 目的：手动压缩（`/compact`）：**让 AI 自己压**——核心只给 compact 工具，不是系统替它总结。 与单 agent 同一条 own-and-return：队列只占"取/交"，模型调用在工作线程上（界面不被阻塞）。 压不动就**如实说**（通知 + 继续用完整上下文），不静默降级、不假装压过。
    pub fn compact(&self, sid: &str) -> Result<Advance, String> {
        let bus = Arc::clone(&self.bus);
        // 身份块必须在**取走会话之前**读；切分计划与模型调用都在工作线程上。
        let identity = self.call({
            let sid = sid.to_string();
            move |core| core.compact_plan(&sid)
        })?;
        let session = self.call({
            let sid = sid.to_string();
            move |core| core.take_single(&sid)
        })?;
        let joined = std::thread::Builder::new()
            .name("solomni-compact".to_string())
            .spawn(move || {
                let mut s = session;
                let made =
                    crate::capabilities::collab::api::compact_session(&mut s, &identity, None);
                (s, made)
            })
            .map_err(|e| format!("起压缩线程失败：{}", e))?
            .join();
        let (s, made) = match joined {
            Ok(x) => x,
            Err(_) => {
                self.call({
                    let sid = sid.to_string();
                    move |core| {
                        core.abort_running(&sid);
                        Ok(())
                    }
                })?;
                return Err("压缩线程崩溃：会话已按落盘转录保留".to_string());
            }
        };
        let made = match made {
            Ok(v) => v,
            Err(err) => {
                self.call({
                    let sid = sid.to_string();
                    move |core| {
                        core.put_single(&sid, s);
                        Ok(())
                    }
                })?;
                let note =
                    SessionEvent::Notice(crate::capabilities::session::api::interrupted_note(
                        &format!("压缩没成功：{}", err),
                    ));
                let head = bus.push(sid, std::slice::from_ref(&note));
                return Ok(Advance { head });
            }
        };
        let Some(c) = made else {
            self.call({
                let sid = sid.to_string();
                move |core| {
                    core.put_single(&sid, s);
                    Ok(())
                }
            })?;
            let note = SessionEvent::Notice(
                "没有可压缩的内容：当前对话已在保留预算内（可调低保留比例或先继续对话）"
                    .to_string(),
            );
            let head = bus.push(sid, std::slice::from_ref(&note));
            return Ok(Advance { head });
        };
        let line_id = c.line.id;
        let events = vec![
            SessionEvent::Transcript(vec![c.line]),
            SessionEvent::Usage {
                usage: c.usage,
                line: line_id,
            },
        ];
        self.call({
            let sid = sid.to_string();
            let events = events.clone();
            move |core| {
                core.put_single(&sid, s);
                core.persister(&sid).persist(&events);
                Ok(())
            }
        })?;
        let head = bus.push(sid, &events);
        Ok(Advance { head })
    }
}

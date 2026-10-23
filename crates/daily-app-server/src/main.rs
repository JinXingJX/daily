use daily_core::DailyRuntime;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, BufReader};

const PROTOCOL_VERSION: u32 = 1;
const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Eq, PartialEq)]
enum FrameRead {
    Complete,
    TooLarge,
    Eof,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestFrame {
    kind: String,
    protocol_version: u32,
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProtocolError {
    code: String,
    message: String,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum OutgoingFrame {
    #[serde(rename_all = "camelCase")]
    Ready {
        protocol_version: u32,
        workspace_root: String,
    },
    Response {
        id: Value,
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<ProtocolError>,
    },
    Event {
        event: String,
        payload: Value,
    },
    Fatal {
        error: ProtocolError,
    },
}

#[derive(Clone)]
struct ProtocolWriter {
    stdout: Arc<Mutex<io::BufWriter<io::Stdout>>>,
}

impl ProtocolWriter {
    fn new() -> Self {
        Self {
            stdout: Arc::new(Mutex::new(io::BufWriter::new(io::stdout()))),
        }
    }

    fn send(&self, frame: &OutgoingFrame) -> Result<(), String> {
        let line = serde_json::to_vec(frame)
            .map_err(|error| format!("Failed to serialize protocol frame: {error}"))?;
        if line.len() > MAX_FRAME_BYTES {
            return Err(format!(
                "Protocol output frame exceeds {MAX_FRAME_BYTES} bytes."
            ));
        }
        let mut stdout = self
            .stdout
            .lock()
            .map_err(|_| "Protocol output is unavailable.".to_string())?;
        stdout
            .write_all(&line)
            .and_then(|_| stdout.write_all(b"\n"))
            .and_then(|_| stdout.flush())
            .map_err(|error| format!("Failed to write protocol frame: {error}"))
    }

    fn error(&self, id: Value, code: &str, message: impl Into<String>) {
        let _ = self.send(&OutgoingFrame::Response {
            id,
            ok: false,
            result: None,
            error: Some(ProtocolError {
                code: code.into(),
                message: message.into(),
            }),
        });
    }
}

#[tokio::main]
async fn main() {
    let writer = ProtocolWriter::new();
    if let Err(error) = run(writer.clone()).await {
        let _ = writer.send(&OutgoingFrame::Fatal {
            error: ProtocolError {
                code: "startup_failed".into(),
                message: error.clone(),
            },
        });
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run(writer: ProtocolWriter) -> Result<(), String> {
    let project_root = project_root_from_args(std::env::args().skip(1))?;
    let event_writer = writer.clone();
    let runtime = DailyRuntime::new(project_root, move |event, payload| {
        event_writer.send(&OutgoingFrame::Event {
            event: event.to_string(),
            payload,
        })
    })?;

    writer.send(&OutgoingFrame::Ready {
        protocol_version: PROTOCOL_VERSION,
        workspace_root: runtime.project_root().display().to_string(),
    })?;

    let mut reader = BufReader::new(tokio::io::stdin());
    let mut frame = Vec::new();
    loop {
        frame.clear();
        match read_protocol_frame(&mut reader, &mut frame, MAX_FRAME_BYTES)
            .await
            .map_err(|error| format!("Failed to read protocol input: {error}"))?
        {
            FrameRead::Eof => break,
            FrameRead::TooLarge => {
                writer.error(
                    Value::Null,
                    "frame_too_large",
                    format!("Protocol frame exceeds {MAX_FRAME_BYTES} bytes."),
                );
                continue;
            }
            FrameRead::Complete => {}
        }

        let request = match serde_json::from_slice::<RequestFrame>(&frame) {
            Ok(request) => request,
            Err(error) => {
                writer.error(Value::Null, "invalid_request", error.to_string());
                continue;
            }
        };
        if request.kind != "request" || request.protocol_version != PROTOCOL_VERSION {
            writer.error(
                request.id,
                "unsupported_protocol",
                format!("Daily app-server requires protocol version {PROTOCOL_VERSION}."),
            );
            continue;
        }
        if request.method == "shutdown" {
            writer.send(&OutgoingFrame::Response {
                id: request.id,
                ok: true,
                result: Some(json!({ "shuttingDown": true })),
                error: None,
            })?;
            break;
        }

        let runtime = runtime.clone();
        let response_writer = writer.clone();
        let response_id = request.id.clone();
        tokio::spawn(async move {
            let response = match runtime.invoke(&request.method, request.params).await {
                Ok(result) => OutgoingFrame::Response {
                    id: request.id,
                    ok: true,
                    result: Some(result),
                    error: None,
                },
                Err(error) => OutgoingFrame::Response {
                    id: request.id,
                    ok: false,
                    result: None,
                    error: Some(ProtocolError {
                        code: error.code().into(),
                        message: error.into(),
                    }),
                },
            };
            if let Err(error) = response_writer.send(&response) {
                if error.starts_with("Protocol output frame exceeds") {
                    response_writer.error(response_id, "response_too_large", error);
                } else {
                    eprintln!("{error}");
                }
            }
        });
    }

    Ok(())
}

async fn read_protocol_frame<R>(
    reader: &mut R,
    frame: &mut Vec<u8>,
    limit: usize,
) -> io::Result<FrameRead>
where
    R: AsyncBufRead + Unpin,
{
    let mut too_large = false;
    loop {
        let buffer = reader.fill_buf().await?;
        if buffer.is_empty() {
            return Ok(if too_large {
                FrameRead::TooLarge
            } else if frame.is_empty() {
                FrameRead::Eof
            } else {
                FrameRead::Complete
            });
        }

        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(buffer.len(), |index| index + 1);
        if !too_large {
            if frame.len().saturating_add(consumed) > limit {
                too_large = true;
                frame.clear();
            } else {
                frame.extend_from_slice(&buffer[..consumed]);
            }
        }
        reader.consume(consumed);

        if newline.is_some() {
            return Ok(if too_large {
                FrameRead::TooLarge
            } else {
                FrameRead::Complete
            });
        }
    }
}

fn project_root_from_args(args: impl Iterator<Item = String>) -> Result<PathBuf, String> {
    let mut args = args.peekable();
    while let Some(argument) = args.next() {
        if argument == "--project-root" {
            return args
                .next()
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from)
                .ok_or_else(|| "--project-root requires a directory path.".to_string());
        }
    }
    Err("Missing required --project-root argument.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_root_is_required() {
        assert!(project_root_from_args(std::iter::empty()).is_err());
        assert!(project_root_from_args(["--project-root".into()].into_iter()).is_err());
        assert_eq!(
            project_root_from_args(["--project-root".into(), "/tmp/example".into()].into_iter())
                .unwrap(),
            PathBuf::from("/tmp/example")
        );
    }

    #[tokio::test]
    async fn protocol_reader_discards_oversized_frame_and_recovers() {
        let input = b"123456789\nok\n";
        let mut reader = BufReader::with_capacity(3, &input[..]);
        let mut frame = Vec::new();

        assert_eq!(
            read_protocol_frame(&mut reader, &mut frame, 8)
                .await
                .unwrap(),
            FrameRead::TooLarge
        );
        assert!(frame.is_empty());
        assert_eq!(
            read_protocol_frame(&mut reader, &mut frame, 8)
                .await
                .unwrap(),
            FrameRead::Complete
        );
        assert_eq!(frame, b"ok\n");
    }

    #[tokio::test]
    async fn protocol_reader_accepts_final_frame_without_newline() {
        let input = b"request";
        let mut reader = BufReader::new(&input[..]);
        let mut frame = Vec::new();

        assert_eq!(
            read_protocol_frame(&mut reader, &mut frame, 8)
                .await
                .unwrap(),
            FrameRead::Complete
        );
        assert_eq!(frame, input);
        frame.clear();
        assert_eq!(
            read_protocol_frame(&mut reader, &mut frame, 8)
                .await
                .unwrap(),
            FrameRead::Eof
        );
    }
}

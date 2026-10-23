//! Local HTTP fixtures. No real credentials or network services are used.
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};

pub(super) async fn serve(
    responses: Vec<(u16, &'static str, String, usize)>,
) -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1internal", listener.local_addr().unwrap());
    (url, serve_on(listener, responses))
}

pub(super) fn serve_on(
    listener: TcpListener,
    responses: Vec<(u16, &'static str, String, usize)>,
) -> JoinHandle<Vec<String>> {
    tokio::spawn(async move {
        let mut requests = Vec::new();
        for (status, content_type, body, extra_length) in responses {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                let read = stream.read(&mut buf).await.unwrap();
                if read == 0 {
                    break;
                }
                bytes.extend_from_slice(&buf[..read]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            requests.push(String::from_utf8(bytes).unwrap());
            // Status 0 drops the connection without answering.
            if status == 0 {
                continue;
            }
            let headers = format!("HTTP/1.1 {status} Test\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()+extra_length);
            stream.write_all(headers.as_bytes()).await.unwrap();
            for chunk in body.as_bytes().chunks(7) {
                stream.write_all(chunk).await.unwrap();
            }
            stream.shutdown().await.unwrap();
        }
        requests
    })
}

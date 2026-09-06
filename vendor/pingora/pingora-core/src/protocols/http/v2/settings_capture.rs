//! Capture client HTTP/2 preface frames before handing the stream to h2.
//!
//! After TLS (or on cleartext h2c), the client sends the connection preface
//! followed by SETTINGS, and typically WINDOW_UPDATE / PRIORITY before the
//! first HEADERS. We buffer those bytes, parse listen-relevant frames, store
//! them by connection key, then re-play the bytes to the h2 handshake.

use crate::protocols::tls::TlsRef;
use crate::protocols::tls::ALPN;
use crate::protocols::{
    GetProxyDigest, GetSocketDigest, GetTimingDigest, Peek, Shutdown, SocketDigest, Ssl,
    TimingDigest, UniqueID, UniqueIDType, IO, Stream,
};
use async_trait::async_trait;
use bytes::BytesMut;
use log::debug;
use std::collections::HashMap;
use std::fmt;
use std::pin::Pin;
use std::sync::{Mutex, OnceLock};
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
const MAX_CAPTURE: usize = 16 * 1024;
/// Stop preface listen after this many frames past the connection preface
/// (SETTINGS + WINDOW_UPDATE + a few PRIORITY is enough for Akamai-H2).
const MAX_PREFACE_FRAMES: usize = 12;

/// One PRIORITY frame observed on the wire (type 0x02).
#[derive(Clone, Debug, Default)]
pub struct ClientH2PriorityFrame {
    pub stream_id: u32,
    pub exclusive: bool,
    pub depends_on: u32,
    pub weight: u8,
}

/// Client H2 preface listen: SETTINGS + WINDOW_UPDATE + PRIORITY.
#[derive(Clone, Debug, Default)]
pub struct ClientH2Settings {
    pub preface_ok: bool,
    pub header_table_size: Option<u32>,
    pub enable_push: Option<u32>,
    pub max_concurrent_streams: Option<u32>,
    pub initial_window_size: Option<u32>,
    pub max_frame_size: Option<u32>,
    pub max_header_list_size: Option<u32>,
    /// All raw (id, value) pairs from the first non-ACK SETTINGS frame (**wire order**)
    pub raw_pairs: Vec<(u16, u32)>,
    pub settings_frame_len: usize,
    /// Connection-level WINDOW_UPDATE increment (stream id 0), if seen before HEADERS
    pub connection_window_update: Option<u32>,
    /// PRIORITY frames (type 0x02) seen before first HEADERS
    pub priority_frames: Vec<ClientH2PriorityFrame>,
    /// Akamai-style PRIORITY segment (`0` or `sid:excl:dep:weight,...`)
    pub priority_fingerprint: String,
    /// Frame type sequence observed in preface listen (e.g. `4,8,2`)
    pub frame_type_sequence: Vec<u8>,
    pub notes: Vec<String>,
}

fn store() -> &'static Mutex<HashMap<String, ClientH2Settings>> {
    static S: OnceLock<Mutex<HashMap<String, ClientH2Settings>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Retrieve (without remove) client H2 SETTINGS for a connection key.
pub fn get_client_h2_settings(conn_key: &str) -> Option<ClientH2Settings> {
    store().lock().ok()?.get(conn_key).cloned()
}

/// Take client H2 SETTINGS for a connection key.
pub fn take_client_h2_settings(conn_key: &str) -> Option<ClientH2Settings> {
    store().lock().ok()?.remove(conn_key)
}

fn put_settings(key: String, s: ClientH2Settings) {
    if let Ok(mut g) = store().lock() {
        if g.len() > 50_000 {
            g.clear();
        }
        debug!(
            "h2 preface key={key} settings={:?} wu={:?} pri_n={} seq={:?}",
            s.raw_pairs,
            s.connection_window_update,
            s.priority_frames.len(),
            s.frame_type_sequence
        );
        g.insert(key, s);
    }
}

fn conn_key_from_stream(s: &dyn IO) -> Option<String> {
    let dig = s.get_socket_digest()?;
    #[cfg(unix)]
    {
        if let Ok(c) = dig.socket_cookie() {
            if c != 0 {
                return Some(format!("sockcookie:{c:x}"));
            }
        }
    }
    let peer = dig.peer_addr()?.as_inet()?;
    let local = dig.local_addr()?.as_inet()?;
    Some(format!(
        "tuple:{}:{}-{}:{}",
        peer.ip(),
        peer.port(),
        local.ip(),
        local.port()
    ))
}

/// Wrap a stream so client H2 preface frames are captured.
pub fn wrap_stream_for_settings_capture(inner: Stream) -> Stream {
    let key = conn_key_from_stream(inner.as_ref());
    Box::new(H2SettingsCaptureStream {
        inner,
        key,
        pending: BytesMut::new(),
        capture: BytesMut::new(),
        done: false,
    })
}

struct H2SettingsCaptureStream {
    inner: Stream,
    key: Option<String>,
    /// Bytes already read for capture that still need to be delivered upstream
    pending: BytesMut,
    capture: BytesMut,
    done: bool,
}

impl H2SettingsCaptureStream {
    fn try_finish_capture(&mut self) {
        if self.done {
            return;
        }
        if self.capture.len() < PREFACE.len() {
            // Wait for more unless clearly not H2
            if !self.capture.is_empty() && !PREFACE.starts_with(&self.capture) {
                self.done = true;
                self.pending.extend_from_slice(&self.capture);
                self.capture.clear();
            }
            return;
        }
        if &self.capture[..PREFACE.len()] != PREFACE {
            self.done = true;
            self.pending.extend_from_slice(&self.capture);
            self.capture.clear();
            return;
        }

        let mut settings = ClientH2Settings {
            preface_ok: true,
            priority_fingerprint: "0".into(),
            ..Default::default()
        };
        let mut offset = PREFACE.len();
        let mut frames = 0usize;
        let mut saw_settings = false;
        let mut stop = false;

        while !stop && frames < MAX_PREFACE_FRAMES {
            if self.capture.len() < offset + 9 {
                // Need more bytes for next frame header
                return;
            }
            let len = ((self.capture[offset] as usize) << 16)
                | ((self.capture[offset + 1] as usize) << 8)
                | (self.capture[offset + 2] as usize);
            let ftype = self.capture[offset + 3];
            let flags = self.capture[offset + 4];
            let stream_id = u32::from_be_bytes([
                self.capture[offset + 5] & 0x7f,
                self.capture[offset + 6],
                self.capture[offset + 7],
                self.capture[offset + 8],
            ]);
            let total = offset + 9 + len;
            if total > MAX_CAPTURE {
                settings
                    .notes
                    .push("preface capture hit MAX_CAPTURE".into());
                break;
            }
            if self.capture.len() < total {
                return;
            }

            settings.frame_type_sequence.push(ftype);
            frames += 1;
            let payload = &self.capture[offset + 9..total];

            match ftype {
                0x4 => {
                    // SETTINGS
                    if !saw_settings {
                        settings.settings_frame_len = len;
                        let ack = flags & 0x1 != 0;
                        if ack {
                            settings.notes.push("first SETTINGS was ACK".into());
                        } else {
                            parse_settings_payload(payload, &mut settings);
                        }
                        saw_settings = true;
                    } else {
                        settings.notes.push("additional SETTINGS in preface".into());
                    }
                }
                0x8 => {
                    // WINDOW_UPDATE
                    if payload.len() >= 4 {
                        let inc = u32::from_be_bytes([
                            payload[0] & 0x7f,
                            payload[1],
                            payload[2],
                            payload[3],
                        ]);
                        if stream_id == 0 {
                            if settings.connection_window_update.is_none() {
                                settings.connection_window_update = Some(inc);
                            }
                        } else {
                            settings.notes.push(format!(
                                "stream WINDOW_UPDATE sid={stream_id} inc={inc}"
                            ));
                        }
                    }
                }
                0x2 => {
                    // PRIORITY (deprecated in RFC 9113 but still sent by some stacks)
                    if payload.len() >= 5 {
                        let dep_word = u32::from_be_bytes([
                            payload[0], payload[1], payload[2], payload[3],
                        ]);
                        let exclusive = (dep_word & 0x8000_0000) != 0;
                        let depends_on = dep_word & 0x7fff_ffff;
                        let weight = payload[4].wrapping_add(1); // wire is weight-1
                        settings.priority_frames.push(ClientH2PriorityFrame {
                            stream_id,
                            exclusive,
                            depends_on,
                            weight,
                        });
                    }
                }
                0x1 | 0x9 | 0x5 => {
                    // HEADERS / CONTINUATION / PUSH_PROMISE — end preface listen
                    settings
                        .notes
                        .push(format!("preface stop at frame type=0x{ftype:02x}"));
                    // Include this frame in delivered bytes; stop parsing further.
                    break;
                }
                0x6 => {
                    // PING — ignore payload, keep listening
                }
                other => {
                    settings
                        .notes
                        .push(format!("preface other frame type=0x{other:02x}"));
                    // After SETTINGS, unknown frames → stop before eating more.
                    if saw_settings {
                        break;
                    }
                }
            }

            offset = total;

            // Do NOT wait across reads for WINDOW_UPDATE — that deadlocks the
            // h2 handshake (client SETTINGS never reaches the codec). Only parse
            // frames already fully present in this buffer after SETTINGS.
            if saw_settings {
                if self.capture.len() < offset + 9 {
                    stop = true;
                } else {
                    let nlen = ((self.capture[offset] as usize) << 16)
                        | ((self.capture[offset + 1] as usize) << 8)
                        | (self.capture[offset + 2] as usize);
                    if self.capture.len() < offset + 9 + nlen {
                        stop = true;
                    }
                    // else: another full frame is buffered; continue loop
                }
            }
        }

        if !saw_settings && frames > 0 {
            settings
                .notes
                .push("no SETTINGS frame in preface window".into());
        }

        // Build Akamai PRIORITY segment
        if settings.priority_frames.is_empty() {
            settings.priority_fingerprint = "0".into();
        } else {
            settings.priority_fingerprint = settings
                .priority_frames
                .iter()
                .map(|p| {
                    format!(
                        "{}:{}:{}:{}",
                        p.stream_id,
                        if p.exclusive { 1 } else { 0 },
                        p.depends_on,
                        p.weight
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
        }

        if let Some(k) = self.key.clone() {
            put_settings(k, settings);
        }

        // Deliver all bytes read so far (preface + frames including stop frame)
        self.pending.extend_from_slice(&self.capture);
        self.capture.clear();
        self.done = true;
    }
}

fn parse_settings_payload(payload: &[u8], out: &mut ClientH2Settings) {
    let mut i = 0;
    while i + 6 <= payload.len() {
        let id = u16::from_be_bytes([payload[i], payload[i + 1]]);
        let val = u32::from_be_bytes([
            payload[i + 2],
            payload[i + 3],
            payload[i + 4],
            payload[i + 5],
        ]);
        out.raw_pairs.push((id, val));
        match id {
            0x1 => out.header_table_size = Some(val),
            0x2 => out.enable_push = Some(val),
            0x3 => out.max_concurrent_streams = Some(val),
            0x4 => out.initial_window_size = Some(val),
            0x5 => out.max_frame_size = Some(val),
            0x6 => out.max_header_list_size = Some(val),
            _ => {}
        }
        i += 6;
    }
}

impl fmt::Debug for H2SettingsCaptureStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("H2SettingsCaptureStream")
            .field("key", &self.key)
            .field("done", &self.done)
            .field("pending", &self.pending.len())
            .finish()
    }
}

impl AsyncRead for H2SettingsCaptureStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        // Serve pending first
        if !self.pending.is_empty() {
            let n = buf.remaining().min(self.pending.len());
            buf.put_slice(&self.pending[..n]);
            let _ = self.pending.split_to(n);
            return Poll::Ready(Ok(()));
        }
        if !self.done {
            // Read more into capture
            let mut tmp = [0u8; 2048];
            let mut rb = ReadBuf::new(&mut tmp);
            match Pin::new(&mut self.inner).poll_read(cx, &mut rb) {
                Poll::Ready(Ok(())) => {
                    let filled = rb.filled();
                    if filled.is_empty() {
                        // EOF during capture
                        self.done = true;
                        let cap = std::mem::take(&mut self.capture);
                        self.pending.extend_from_slice(&cap);
                        return Poll::Ready(Ok(()));
                    }
                    self.capture.extend_from_slice(filled);
                    if self.capture.len() > MAX_CAPTURE {
                        self.done = true;
                        let cap = std::mem::take(&mut self.capture);
                        self.pending.extend_from_slice(&cap);
                    } else {
                        self.try_finish_capture();
                    }
                    // Loop once: if we now have pending, serve it
                    if !self.pending.is_empty() {
                        let n = buf.remaining().min(self.pending.len());
                        buf.put_slice(&self.pending[..n]);
                        let _ = self.pending.split_to(n);
                        return Poll::Ready(Ok(()));
                    }
                    // Need more capture data
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for H2SettingsCaptureStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[async_trait]
impl Shutdown for H2SettingsCaptureStream {
    async fn shutdown(&mut self) {
        self.inner.shutdown().await
    }
}

impl UniqueID for H2SettingsCaptureStream {
    fn id(&self) -> UniqueIDType {
        self.inner.id()
    }
}

impl Ssl for H2SettingsCaptureStream {
    fn get_ssl(&self) -> Option<&TlsRef> {
        self.inner.get_ssl()
    }
    fn get_ssl_digest(&self) -> Option<std::sync::Arc<crate::protocols::tls::SslDigest>> {
        self.inner.get_ssl_digest()
    }
    fn selected_alpn_proto(&self) -> Option<ALPN> {
        self.inner.selected_alpn_proto()
    }
}

impl GetTimingDigest for H2SettingsCaptureStream {
    fn get_timing_digest(&self) -> Vec<Option<TimingDigest>> {
        self.inner.get_timing_digest()
    }
}

impl GetProxyDigest for H2SettingsCaptureStream {
    fn get_proxy_digest(
        &self,
    ) -> Option<std::sync::Arc<crate::protocols::raw_connect::ProxyDigest>> {
        self.inner.get_proxy_digest()
    }
}

impl GetSocketDigest for H2SettingsCaptureStream {
    fn get_socket_digest(&self) -> Option<std::sync::Arc<SocketDigest>> {
        self.inner.get_socket_digest()
    }
    fn set_socket_digest(&mut self, socket_digest: SocketDigest) {
        self.inner.set_socket_digest(socket_digest)
    }
}

#[async_trait]
impl Peek for H2SettingsCaptureStream {
    async fn try_peek(&mut self, buf: &mut [u8]) -> std::io::Result<bool> {
        self.inner.try_peek(buf).await
    }
}

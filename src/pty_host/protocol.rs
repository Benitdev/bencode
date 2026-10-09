//! What the app, the attach clients and the host say to each other: frames
//! of a tag, a big-endian length and a payload. Structured payloads are
//! JSON. A connection opens with `Hello` both ways; an old host answers
//! with its own version, and a client that cannot speak it says so.

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

/// Bumped when a frame's meaning changes; a host and a client of different
/// versions refuse each other.
pub const PROTOCOL_VERSION: u32 = 1;

/// A frame's payload is at most this; output is sent in smaller chunks.
const MAX_PAYLOAD: u32 = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    Hello(u32),
    /// Client: show session `id`, creating it when it does not exist.
    Attach(Vec<u8>),
    /// Client: keys for the shell.
    Input(Vec<u8>),
    /// Client: the terminal's size, columns then rows.
    Resize(u16, u16),
    /// App: a `ControlRequest`.
    Control(Vec<u8>),
    /// Host: the session is shown; an `Attached`.
    Attached(Vec<u8>),
    /// Host: what the shell wrote.
    Output(Vec<u8>),
    /// Host: the shell ended with this status.
    Exited(i32),
    /// Host: a `ControlReply`.
    ControlReply(Vec<u8>),
    Error(String),
}

/// `Attach`'s payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachRequest {
    pub id: String,
    /// Where a new session's shell starts.
    pub cwd: String,
    /// Added to a new session's environment.
    pub env: Vec<(String, String)>,
    pub cols: u16,
    pub rows: u16,
}

/// `Attached`'s payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attached {
    /// The process the session started (`login` on macOS).
    pub pid: u32,
    /// A new session, rather than one shown again.
    pub created: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlRequest {
    List,
    Kill { ids: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlReply {
    Sessions(Vec<SessionInfo>),
    Done,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub pid: u32,
    /// Its shell still runs.
    pub alive: bool,
}

fn tag(frame: &Frame) -> u8 {
    match frame {
        Frame::Hello(_) => 1,
        Frame::Attach(_) => 2,
        Frame::Input(_) => 3,
        Frame::Resize(..) => 4,
        Frame::Control(_) => 5,
        Frame::Attached(_) => 10,
        Frame::Output(_) => 11,
        Frame::Exited(_) => 12,
        Frame::ControlReply(_) => 13,
        Frame::Error(_) => 14,
    }
}

pub fn write_frame(out: &mut impl Write, frame: &Frame) -> io::Result<()> {
    let payload: Vec<u8> = match frame {
        Frame::Hello(version) => version.to_be_bytes().to_vec(),
        Frame::Resize(cols, rows) => [cols.to_be_bytes(), rows.to_be_bytes()].concat(),
        Frame::Exited(code) => code.to_be_bytes().to_vec(),
        Frame::Error(message) => message.as_bytes().to_vec(),
        Frame::Attach(body)
        | Frame::Input(body)
        | Frame::Control(body)
        | Frame::Attached(body)
        | Frame::Output(body)
        | Frame::ControlReply(body) => body.clone(),
    };
    let mut bytes = Vec::with_capacity(5 + payload.len());
    bytes.push(tag(frame));
    bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&payload);
    out.write_all(&bytes)?;
    out.flush()
}

/// The next frame; `None` at a clean end of the stream.
pub fn read_frame(input: &mut impl Read) -> io::Result<Option<Frame>> {
    let mut head = [0u8; 5];
    match input.read_exact(&mut head) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(err) => return Err(err),
    }
    let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]);
    if len > MAX_PAYLOAD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("a {len}-byte frame"),
        ));
    }
    let mut payload = vec![0u8; len as usize];
    input.read_exact(&mut payload)?;
    let bad = |what: &str| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("a malformed {what} frame"),
        )
    };
    let frame = match head[0] {
        1 => Frame::Hello(u32::from_be_bytes(
            payload.try_into().map_err(|_| bad("Hello"))?,
        )),
        2 => Frame::Attach(payload),
        3 => Frame::Input(payload),
        4 => {
            let [c0, c1, r0, r1]: [u8; 4] = payload.try_into().map_err(|_| bad("Resize"))?;
            Frame::Resize(u16::from_be_bytes([c0, c1]), u16::from_be_bytes([r0, r1]))
        }
        5 => Frame::Control(payload),
        10 => Frame::Attached(payload),
        11 => Frame::Output(payload),
        12 => Frame::Exited(i32::from_be_bytes(
            payload.try_into().map_err(|_| bad("Exited"))?,
        )),
        13 => Frame::ControlReply(payload),
        14 => Frame::Error(String::from_utf8_lossy(&payload).into_owned()),
        other => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("an unknown frame {other}"),
            ));
        }
    };
    Ok(Some(frame))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_frame_round_trips() {
        let frames = [
            Frame::Hello(PROTOCOL_VERSION),
            Frame::Attach(b"{}".to_vec()),
            Frame::Input(b"ls\r".to_vec()),
            Frame::Resize(120, 40),
            Frame::Control(b"\"List\"".to_vec()),
            Frame::Attached(b"{}".to_vec()),
            Frame::Output(vec![0, 27, 255]),
            Frame::Exited(-1),
            Frame::ControlReply(Vec::new()),
            Frame::Error("no".into()),
        ];
        let mut wire = Vec::new();
        for frame in &frames {
            write_frame(&mut wire, frame).unwrap();
        }
        let mut input = wire.as_slice();
        for frame in &frames {
            assert_eq!(read_frame(&mut input).unwrap().as_ref(), Some(frame));
        }
        assert_eq!(read_frame(&mut input).unwrap(), None);
    }

    #[test]
    fn a_truncated_or_unknown_frame_is_an_error() {
        let mut wire = Vec::new();
        write_frame(&mut wire, &Frame::Input(b"abc".to_vec())).unwrap();
        assert!(read_frame(&mut &wire[..wire.len() - 1]).is_err());
        assert!(read_frame(&mut &[99u8, 0, 0, 0, 0][..]).is_err());
    }
}

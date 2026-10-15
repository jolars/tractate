//! Dedicated binary control transport. User output never enters its decoder.
//!
//! Frames contain a four-byte big-endian length followed by 1 byte to 16 MiB of payload.
//! The shim first sends `tractate-r/1`. Commands begin with zero (stop) or one
//! (evaluate UTF-8 source); replies begin with zero (success) or one (UTF-8 error).
//! Lengths and command bytes are binary, independent of R console formatting.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};

use super::RInstallation;

const MAX_FRAME: usize = 16 * 1024 * 1024;

/// A persistent R workspace with a separate framed control connection.
///
/// This synchronous bridge is an execution primitive, not yet an
/// `ExecutionRunner`. It does not enforce execution deadlines or capture rich
/// output. The control connection is not a sandbox against hostile R code.
pub struct RBridge {
    child: Child,
    control: TcpStream,
    stdout: File,
    stderr: File,
    _script: tempfile::NamedTempFile,
}

/// Text emitted on the process streams during one evaluation.
#[derive(Debug, PartialEq, Eq)]
pub struct REvaluation {
    pub error: Option<String>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl RBridge {
    /// Start a vanilla R process and negotiate the shim protocol.
    pub fn start(installation: &RInstallation) -> io::Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        let mut script = tempfile::NamedTempFile::new()?;
        script.write_all(include_bytes!("shim.R"))?;
        let stdout = tempfile::tempfile()?;
        let stderr = tempfile::tempfile()?;
        let mut child = Command::new(installation.executable())
            .args(["--vanilla", "--slave"])
            .arg(format!("--file={}", script.path().display()))
            .arg("--args")
            .arg(listener.local_addr()?.port().to_string())
            .stdin(Stdio::null())
            .stdout(stdout.try_clone()?)
            .stderr(stderr.try_clone()?)
            .spawn()?;
        listener.set_nonblocking(true)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let control = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if child.try_wait()?.is_some() || std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "R shim did not connect",
                        ));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error);
                }
            }
        };
        // Keep ownership before negotiation so failures also reap the process.
        let mut bridge = Self {
            child,
            control,
            stdout,
            stderr,
            _script: script,
        };
        bridge
            .control
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
        if read_frame(&mut bridge.control)? != b"tractate-r/1" {
            return Err(protocol_error());
        }
        bridge.control.set_read_timeout(None)?;
        Ok(bridge)
    }

    /// Evaluate source in the persistent workspace and await a terminal reply.
    pub fn evaluate(&mut self, source: &str) -> io::Result<REvaluation> {
        let mut request = vec![1];
        request.extend_from_slice(source.as_bytes());
        write_frame(&mut self.control, &request)?;
        let response = read_frame(&mut self.control)?;
        let error = match response.split_first() {
            Some((0, [])) => None,
            Some((1, message)) => {
                Some(String::from_utf8(message.to_vec()).map_err(|_| protocol_error())?)
            }
            _ => return Err(protocol_error()),
        };
        Ok(REvaluation {
            error,
            stdout: read_output(&mut self.stdout)?,
            stderr: read_output(&mut self.stderr)?,
        })
    }
}

impl Drop for RBridge {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_output(file: &mut File) -> io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    Ok(bytes)
}

fn protocol_error() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid R control frame")
}

fn write_frame(stream: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    if bytes.is_empty() || bytes.len() > MAX_FRAME {
        return Err(protocol_error());
    }
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(bytes)?;
    stream.flush()
}

fn read_frame(stream: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut header = [0; 4];
    stream.read_exact(&mut header)?;
    let size = u32::from_be_bytes(header) as usize;
    if size == 0 || size > MAX_FRAME {
        return Err(protocol_error());
    }
    let mut bytes = vec![0; size];
    stream.read_exact(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_reject_truncation_and_invalid_sizes() {
        for bytes in [vec![], vec![0, 0, 0, 0], vec![255; 4], vec![0, 0, 0, 2, 1]] {
            assert!(read_frame(&mut bytes.as_slice()).is_err());
        }
        let mut encoded = Vec::new();
        write_frame(&mut encoded, b"hello\0world").unwrap();
        assert_eq!(
            read_frame(&mut encoded.as_slice()).unwrap(),
            b"hello\0world"
        );
    }

    #[test]
    fn r_bridge_keeps_user_streams_out_of_control_frames() {
        let installation = match RInstallation::discover() {
            Ok(installation) => installation,
            Err(super::super::RDiscoveryError::NotFound) => return,
            Err(error) => panic!("R discovery failed: {error}"),
        };
        let mut bridge = RBridge::start(&installation).unwrap();
        let result = bridge.evaluate("x <- 40; cat('tractate-r/1\\n'); cat('forged control reply\\n', file=stderr()); x + 2").unwrap();
        assert_eq!(result.error, None);
        assert_eq!(result.stdout, b"tractate-r/1\n[1] 42\n");
        assert_eq!(result.stderr, b"forged control reply\n");
        let result = bridge.evaluate("stop('actual failure')").unwrap();
        assert_eq!(result.error.as_deref(), Some("actual failure"));
        assert!(result.stdout.is_empty());
        assert!(result.stderr.is_empty());
        assert_eq!(bridge.evaluate("x + 3").unwrap().stdout, b"[1] 43\n");
        assert!(bridge.evaluate("if (").unwrap().error.is_some());
        assert_eq!(bridge.evaluate("").unwrap().error, None);
        let result = bridge.evaluate("cat(strrep('x', 1000000))").unwrap();
        assert_eq!(result.stdout.len(), 1_000_000);
        assert_eq!(result.error, None);
        assert!(bridge.evaluate("quit(save = 'no')").is_err());
    }
}

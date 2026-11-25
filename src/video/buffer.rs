//! Bounded decoded pictures, retained for short movies and consumed as a queue for long ones.

use std::collections::VecDeque;
use std::sync::mpsc::{sync_channel, Receiver, TryRecvError};
use std::sync::Arc;

use or_media::{MediaError, VideoStream};

use super::{sized, FRAME_BYTES, MAX_SPEED};

const BUFFER_BYTES: usize = 128 * 1024 * 1024;
// Reserve four scaled pictures for the channel, conversion output and display.
const BUFFER_FRAMES: usize = BUFFER_BYTES / FRAME_BYTES - 4;
const AHEAD_SECONDS: f64 = 2.0;
const SHORT_SECONDS: f64 = 10.0;

type Picture = Arc<[u8]>;

pub(super) struct Decoder {
    pub(super) bytes: Arc<[u8]>,
    receiver: Option<Receiver<Result<Picture, MediaError>>>,
    pictures: VecDeque<Picture>,
    first: u64,
    total: u64,
    capacity: usize,
    retain: bool,
    ended: bool,
}

fn capacity(frames: u64, fps: f64) -> (usize, bool) {
    let retain =
        frames > 0 && frames <= BUFFER_FRAMES as u64 && frames as f64 / fps <= SHORT_SECONDS;
    let capacity = if retain {
        frames as usize
    } else {
        ((fps * MAX_SPEED * AHEAD_SECONDS).ceil() as usize).clamp(2, BUFFER_FRAMES)
    };
    (capacity, retain)
}

impl Decoder {
    pub(super) fn start(
        bytes: Arc<[u8]>,
        mut stream: Box<dyn VideoStream>,
        first: u64,
        total: u64,
        fps: f64,
    ) -> Self {
        let (capacity, retain) = capacity(total, fps);
        let first = if retain { 0 } else { first };
        let (sender, receiver) = sync_channel(2);
        std::thread::spawn(move || {
            if first > 0 {
                if let Err(why) = stream.seek(first) {
                    let _ = sender.send(Err(why));
                    return;
                }
            }
            loop {
                let sent = match stream.next_frame() {
                    Ok(Some(frame)) => sender.send(Ok(sized(frame).into())).is_ok(),
                    Ok(None) => false,
                    Err(why) => {
                        let _ = sender.send(Err(why));
                        false
                    }
                };
                if !sent {
                    break;
                }
            }
        });
        Self {
            bytes,
            receiver: Some(receiver),
            pictures: VecDeque::new(),
            first,
            total,
            capacity,
            retain,
            ended: false,
        }
    }

    pub(super) fn pump(&mut self) -> Result<(), MediaError> {
        while self.pictures.len() < self.capacity {
            let Some(receiver) = &self.receiver else {
                break;
            };
            match receiver.try_recv() {
                Ok(Ok(frame)) => self.pictures.push_back(frame),
                Ok(Err(why)) => {
                    self.receiver = None;
                    self.ended = true;
                    return Err(why);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.receiver = None;
                    self.ended = true;
                    break;
                }
            }
        }
        if self.total > 0 && self.first + self.pictures.len() as u64 >= self.total {
            self.receiver = None;
            self.ended = true;
        }
        Ok(())
    }

    pub(super) fn last_frame(&self) -> Option<u64> {
        self.total.checked_sub(1)
    }

    pub(super) fn can_seek(&self, bytes: &Arc<[u8]>, at: u64) -> bool {
        Arc::ptr_eq(&self.bytes, bytes)
            && (self.retain || (self.first..self.first + self.pictures.len() as u64).contains(&at))
    }

    pub(super) fn frame(&mut self, at: u64) -> Option<Picture> {
        let offset = usize::try_from(at.checked_sub(self.first)?).ok()?;
        let frame = Arc::clone(self.pictures.get(offset)?);
        if !self.retain {
            self.pictures.drain(..=offset);
            self.first = at + 1;
        }
        Some(frame)
    }

    pub(super) fn finished(&self, at: u64) -> bool {
        self.ended && at >= self.first + self.pictures.len() as u64
    }

    pub(super) fn filling(&self) -> bool {
        self.receiver.is_some() && self.pictures.len() < self.capacity
    }

    pub(super) fn progress(&self, at: u64, fps: f64, speed: f64) -> f32 {
        if self.ended {
            return 1.0;
        }
        let (ready, target) = if self.retain {
            (self.pictures.len() as u64, self.total)
        } else {
            let ready = (self.first + self.pictures.len() as u64).saturating_sub(at);
            let wanted =
                ((fps * speed * AHEAD_SECONDS).ceil() as u64).clamp(1, self.capacity as u64);
            let target = if self.total > 0 {
                wanted.min(self.total.saturating_sub(at))
            } else {
                wanted
            };
            (ready, target)
        };
        if target == 0 {
            1.0
        } else {
            (ready as f32 / target as f32).min(1.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_movies_fit_whole_and_long_or_fast_movies_stay_bounded() {
        assert_eq!(capacity(115, 30.0), (115, true));
        for (frames, fps) in [(900, 30.0), (u64::MAX, 240.0), (200, 1.0), (0, 30.0)] {
            let (count, retain) = capacity(frames, fps);
            assert!(!retain);
            assert!(count >= 2);
            assert!((count + 4) * FRAME_BYTES <= BUFFER_BYTES);
        }
    }

    #[test]
    fn streaming_frames_are_consumed_but_cached_frames_survive_seeks() {
        for retain in [false, true] {
            let bytes: Arc<[u8]> = Arc::from(&b"movie"[..]);
            let mut decoder = Decoder {
                bytes: Arc::clone(&bytes),
                receiver: None,
                pictures: (0..3).map(|v| Arc::from(vec![v])).collect(),
                first: 0,
                total: 3,
                capacity: 3,
                retain,
                ended: true,
            };
            assert_eq!(decoder.frame(1).as_deref(), Some(&[1][..]));
            assert_eq!(decoder.can_seek(&bytes, 0), retain);
            assert_eq!(decoder.pictures.len(), if retain { 3 } else { 1 });
            assert!(!decoder.can_seek(&Arc::from(&b"other"[..]), 0));
            assert!(decoder.finished(3));
        }
    }
}

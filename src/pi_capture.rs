#![allow(dead_code)]

use anyhow::Result;

#[derive(Debug, Clone)]
pub struct FrameMeta {
    pub width: u32,
    pub height: u32,
    pub pixel_format: String,
    pub timestamp_ns: u64,
    pub frame_index: u64,
}

#[derive(Debug, Clone)]
pub struct CaptureConfig {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub pixel_format: String,
}

pub trait FrameConsumer {
    fn on_frame(&mut self, meta: &FrameMeta, yuv420_frame: &[u8]) -> Result<()>;
}

pub trait PiFrameSource {
    fn start(&mut self, cfg: &CaptureConfig) -> Result<()>;
    fn pump(&mut self, consumer: &mut dyn FrameConsumer) -> Result<()>;
    fn stop(&mut self) -> Result<()>;
}

pub struct LibcameraContract;

impl LibcameraContract {
    pub fn adapter_notes() -> &'static str {
        "Implement this contract with a libcamera callback adapter:\n\
         1) Configure stream as YUV420.\n\
         2) For each frame, flatten planes into contiguous YUV420 buffer.\n\
         3) Call FrameConsumer::on_frame(meta, yuv420).\n\
         4) Keep callback latency bounded; avoid blocking camera thread."
    }
}

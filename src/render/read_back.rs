//! A texture's pixels copied back from the GPU.

/// The top-left `w` by `h` texels of an RGBA8 `texture` made with `COPY_SRC`, copied back from the GPU.
pub fn rgba(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    w: u32,
    h: u32,
) -> Result<image::RgbaImage, String> {
    // ⚠ A buffer copy pads every row to 256 bytes.
    let row =
        (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    // What a lost device refuses of these steps is this readback's error and no panic.
    let refused = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("texture readback"),
        size: u64::from(row) * u64::from(h),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    if let Some(error) = pollster::block_on(refused.pop()) {
        return Err(format!("the readback was refused: {error}"));
    }
    // ⚠ wgpu panics inside a poll that meets a lost device, whatever is asked of it, and a poll
    // is where a device hung by the frame before is found lost. That panic is this readback's
    // error; no hook reports it meanwhile.
    let wait = wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    };
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let polled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.poll(wait)));
    std::panic::set_hook(hook);
    if polled.is_err() {
        return Err("the graphics device was lost during the readback".into());
    }
    // A poll that waited has called back: nothing waits on a device that will not.
    rx.try_recv()
        .map_err(|e| format!("the readback never finished: {e}"))?
        .map_err(|e| format!("the readback failed: {e}"))?;
    let mapped = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for line in mapped.chunks(row as usize).take(h as usize) {
        pixels.extend_from_slice(&line[..(w * 4) as usize]);
    }
    drop(mapped);
    buffer.unmap();
    image::RgbaImage::from_raw(w, h, pixels)
        .ok_or_else(|| "the readback was shorter than the frame".into())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_readback_on_a_lost_device_is_an_error_and_no_panic() {
        const WHAT: &str = "a_readback_on_a_lost_device_is_an_error_and_no_panic";
        let Some((device, queue)) = crate::render::test_device(WHAT) else {
            return;
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let read = || super::rgba(&device, &queue, &texture, 4, 4);
        assert!(read().is_ok(), "on a device that is there");
        // A device destroyed is a device lost, and its first poll is the one that says so.
        device.destroy();
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        assert!(read().is_err());
    }
}

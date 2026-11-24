use super::*;

/// A device with the timer's features, or a recorded skip.
fn gpu(test: &str) -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: crate::instance::software_asked(),
    })) {
        Ok(adapter) => adapter,
        Err(e) => {
            or_test_support::skip_device(test, format_args!("no wgpu adapter: {e}"));
            return None;
        }
    };
    if !adapter.features().contains(FEATURES) {
        or_test_support::skip_device(test, format_args!("the adapter has no timestamp queries"));
        return None;
    }
    match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("gpu-timer-test"),
        required_features: FEATURES,
        required_limits: adapter.limits(),
        ..Default::default()
    })) {
        Ok(pair) => Some(pair),
        Err(e) => {
            or_test_support::skip_device(test, format_args!("device init failed: {e}"));
            None
        }
    }
}

/// Clears a large target `times` over, which takes the device a while.
fn clear(device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, times: usize) {
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 4096,
            height: 4096,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    for _ in 0..times {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
}

/// Submit `encoder` and read the timer back.
fn read(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    timer: &GpuTimer,
    encoder: wgpu::CommandEncoder,
) {
    queue.submit([encoder.finish()]);
    for _ in 0..1000 {
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the device finishes");
        let _ = timer.collect(device);
        if timer.phase.get() == Phase::Idle {
            return;
        }
    }
    panic!("the timestamps were never read");
}

#[test]
fn a_stage_that_draws_more_takes_longer_and_the_stages_come_back_in_order() {
    let Some((device, queue)) =
        gpu("a_stage_that_draws_more_takes_longer_and_the_stages_come_back_in_order")
    else {
        return;
    };
    let timer = GpuTimer::new(&device, &queue).expect("the device has the features");
    let mut encoder = device.create_command_encoder(&Default::default());
    timer.begin(&mut encoder);
    clear(&device, &mut encoder, 16);
    timer.mark(&mut encoder, "heavy");
    clear(&device, &mut encoder, 1);
    timer.mark(&mut encoder, "light");
    timer.end(&mut encoder);
    read(&device, &queue, &timer, encoder);

    let stages = timer.last();
    println!("{stages:?}");
    let names: Vec<_> = stages.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, ["heavy", "light"]);
    let (heavy, light) = (stages[0].1, stages[1].1);
    assert!(light > 0.0, "{stages:?}");
    // ⚠ Longer and no more: a software adapter took 2.5 times as long for sixteen times the clears.
    assert!(heavy > light, "sixteen clears against one: {stages:?}");
}

#[test]
fn a_frame_begun_while_the_last_is_unread_leaves_the_last_to_be_read() {
    let Some((device, queue)) =
        gpu("a_frame_begun_while_the_last_is_unread_leaves_the_last_to_be_read")
    else {
        return;
    };
    let timer = GpuTimer::new(&device, &queue).expect("the device has the features");
    let mut first = device.create_command_encoder(&Default::default());
    timer.begin(&mut first);
    timer.mark(&mut first, "first");
    timer.end(&mut first);
    // The second begins before the first was read: it records nothing and keeps the first's stages
    let mut second = device.create_command_encoder(&Default::default());
    timer.begin(&mut second);
    timer.mark(&mut second, "second");
    timer.end(&mut second);
    assert_eq!(*timer.marks.borrow(), ["first"]);
    queue.submit([first.finish(), second.finish()]);
    for _ in 0..1000 {
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the device finishes");
        timer.collect(&device);
        if timer.phase.get() == Phase::Idle {
            break;
        }
    }
    assert_eq!(timer.last().len(), 1);
    assert_eq!(timer.last()[0].0, "first");
}

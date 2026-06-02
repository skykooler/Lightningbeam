# Plan for Async Rendering Helpers

I'm creating this temporary document to plan the async rendering changes.

## Current Architecture (Synchronous)
`render_frame_to_rgba_hdr()` in video_exporter.rs:
1. Render document to RGBA (lines 750-991)
2. GPU YUV conversion (lines 993-1005)
3. Copy YUV to staging buffer (lines 1007-1029)
4. Submit GPU commands (line 1031)
5. **BLOCKING** map_async + wait (lines 1033-1045)
6. Extract Y, U, V planes from mapped buffer (lines 1047-1087)
7. Unmap and return YUV planes (lines 1089-1092)

## New Architecture (Async Pipelined)
Split into two phases using ReadbackPipeline:

### Phase 1: Submit Frame (Non-blocking)
New function `submit_frame_to_readback_pipeline()`:
- Input: buffer from ReadbackPipeline.acquire()
- Steps 1-3: Render to RGBA, GPU YUV, copy to buffer's YUV texture
- Return encoder to ReadbackPipeline for submission
- **Does NOT wait for GPU**

### Phase 2: Extract YUV (After async mapping)
Helper function `extract_yuv_planes_from_buffer()`:
- Input: mapped buffer data from ReadbackPipeline
- Steps 6-7: Extract Y, U, V planes, return them
- Used after ReadbackPipeline.get_mapped_data()

## Modified render_next_video_frame()
New async pipeline loop:
```
while more_work_to_do:
    // Poll for completed frames
    for result in pipeline.poll_nonblocking():
        data = pipeline.get_mapped_data(result.buffer_id)
        (y, u, v) = extract_yuv_planes(data)
        send_to_encoder_in_order(result.frame_num, y, u, v)
        pipeline.release(result.buffer_id)

    // Submit new frames (up to 3 in flight)
    if current_frame < total_frames && frames_in_flight < 3:
        if let Some(buffer) = pipeline.acquire(frame_num, timestamp):
            encoder = submit_frame_to_pipeline(buffer)
            pipeline.submit_and_readback(buffer.id, encoder)
            frames_in_flight++
            current_frame++

    // Done when all frames submitted AND all completed
    if current_frame >= total_frames && frames_in_flight == 0:
        return Ok(false)

    return Ok(true)  // More work to do
```

This achieves triple buffering:
- Frame N: GPU rendering
- Frame N-1: GPU→CPU async transfer
- Frame N-2: CPU encoding

Expected speedup: 5x

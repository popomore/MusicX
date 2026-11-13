//! AVPlayer pass-through audio tap. Playback buffers are always returned untouched.
use super::Capture;
use block2::RcBlock;
use dispatch2::MainThreadBound;
use objc2::{MainThreadMarker, rc::Retained};
use objc2_av_foundation::{
    AVAssetTrack, AVAudioMixInputParameters, AVMediaTypeAudio, AVMutableAudioMix,
    AVMutableAudioMixInputParameters, AVPlayerItem,
};
use objc2_core_audio_types::{
    AudioBuffer, AudioBufferList, AudioStreamBasicDescription, kAudioFormatFlagIsBigEndian,
    kAudioFormatFlagIsFloat, kAudioFormatFlagIsNonInterleaved, kAudioFormatFlagIsPacked,
    kAudioFormatFlagIsSignedInteger, kAudioFormatLinearPCM,
};
use objc2_core_foundation::CFRetained;
use objc2_foundation::{NSArray, NSError};
use objc2_media_toolbox::*;
use std::{
    ffi::c_void,
    ptr::NonNull,
    sync::{Arc, atomic::Ordering},
};

pub(super) fn load_track(item: &Retained<AVPlayerItem>, capture: Arc<Capture>) {
    let Some(media_type) = (unsafe { AVMediaTypeAudio }) else {
        capture.unavailable();
        return;
    };
    // Only track IDs cross the completion/main-thread boundary. Neither the
    // main-thread-only player item nor a borrowed track crosses threads.
    let bound = Arc::new(MainThreadBound::new(
        item.clone(),
        MainThreadMarker::new().unwrap(),
    ));
    let completion = RcBlock::new(move |tracks: *mut NSArray<AVAssetTrack>, _: *mut NSError| {
        let track_id = unsafe {
            tracks
                .as_ref()
                .and_then(|tracks| tracks.firstObject())
                .map(|track| track.trackID())
        };
        let capture = capture.clone();
        let bound = bound.clone();
        dispatch2::DispatchQueue::main().exec_async(move || {
            // Reject delayed track loads belonging to a replaced item.
            if !super::CURRENT
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &capture))
            {
                return;
            }
            if let Some(track_id) = track_id {
                capture.install(bound.get(MainThreadMarker::new().unwrap()), track_id);
            } else {
                capture.unavailable();
            }
        });
    });
    unsafe {
        item.asset()
            .loadTracksWithMediaType_completionHandler(media_type, &completion)
    };
}

pub(super) fn install(item: &AVPlayerItem, track_id: i32, capture: Arc<Capture>) -> bool {
    let context = Box::into_raw(Box::new(Context {
        capture,
        format: empty_format(),
    }));
    let mut callbacks = MTAudioProcessingTapCallbacks {
        version: kMTAudioProcessingTapCallbacksVersion_0,
        clientInfo: context.cast(),
        init: Some(initialize),
        finalize: Some(finalize),
        prepare: Some(prepare),
        unprepare: Some(unprepare),
        process: Some(process),
    };
    let mut tap = std::ptr::null();
    let result = unsafe {
        MTAudioProcessingTap::create(
            None,
            NonNull::from(&mut callbacks),
            kMTAudioProcessingTapCreationFlag_PostEffects,
            NonNull::from(&mut tap),
        )
    };
    if result != 0 || tap.is_null() {
        // Creation failed, so ownership was not transferred to a tap.
        unsafe { drop(Box::from_raw(context)) };
        return false;
    }
    let tap = unsafe { CFRetained::from_raw(NonNull::new_unchecked(tap.cast_mut())) };
    unsafe {
        let parameters = AVMutableAudioMixInputParameters::audioMixInputParameters();
        parameters.setTrackID(track_id);
        parameters.setAudioTapProcessor(Some(&tap));
        let mix = AVMutableAudioMix::audioMix();
        let parameters: &AVAudioMixInputParameters = &parameters;
        mix.setInputParameters(&NSArray::from_slice(&[parameters]));
        item.setAudioMix(Some(&mix));
    }
    true
}

struct Context {
    capture: Arc<Capture>,
    format: AudioStreamBasicDescription,
}

fn empty_format() -> AudioStreamBasicDescription {
    AudioStreamBasicDescription {
        mSampleRate: 0.0,
        mFormatID: 0,
        mFormatFlags: 0,
        mBytesPerPacket: 0,
        mFramesPerPacket: 0,
        mBytesPerFrame: 0,
        mChannelsPerFrame: 0,
        mBitsPerChannel: 0,
        mReserved: 0,
    }
}

unsafe extern "C-unwind" fn initialize(
    _: NonNull<MTAudioProcessingTap>,
    info: *mut c_void,
    mut storage: NonNull<*mut c_void>,
) {
    unsafe { *storage.as_mut() = info };
}

unsafe extern "C-unwind" fn finalize(tap: NonNull<MTAudioProcessingTap>) {
    unsafe {
        drop(Box::from_raw(
            tap.as_ref().storage().as_ptr().cast::<Context>(),
        ))
    };
}

unsafe extern "C-unwind" fn prepare(
    tap: NonNull<MTAudioProcessingTap>,
    _: isize,
    format: NonNull<AudioStreamBasicDescription>,
) {
    // MediaToolbox serializes prepare/process/unprepare for this tap.
    let context = unsafe { &mut *tap.as_ref().storage().as_ptr().cast::<Context>() };
    context.format = unsafe { *format.as_ref() };
    context.capture.rate.store(
        (context.format.mSampleRate as f32).to_bits(),
        Ordering::Release,
    );
    context.capture.format.store(
        if supported(&context.format) { 1 } else { 2 },
        Ordering::Relaxed,
    );
    context.capture.invalidate();
}

unsafe extern "C-unwind" fn unprepare(tap: NonNull<MTAudioProcessingTap>) {
    let context = unsafe { &*tap.as_ref().storage().as_ptr().cast::<Context>() };
    context.capture.invalidate();
}

unsafe extern "C-unwind" fn process(
    tap: NonNull<MTAudioProcessingTap>,
    frames: isize,
    _: MTAudioProcessingTapFlags,
    buffers: NonNull<AudioBufferList>,
    mut frames_out: NonNull<isize>,
    flags_out: NonNull<MTAudioProcessingTapFlags>,
) {
    let tap = unsafe { tap.as_ref() };
    let result = unsafe {
        tap.source_audio(
            frames,
            buffers,
            flags_out.as_ptr(),
            std::ptr::null_mut(),
            frames_out.as_ptr(),
        )
    };
    if result != 0 {
        unsafe { *frames_out.as_mut() = 0 };
        return;
    }
    let context = unsafe { &*tap.storage().as_ptr().cast::<Context>() };
    if unsafe { *flags_out.as_ptr() } & kMTAudioProcessingTapFlag_StartOfStream != 0 {
        context.capture.invalidate();
    }
    if context.capture.format.load(Ordering::Relaxed) != 1 || !context.capture.active() {
        return;
    }
    let list = unsafe { buffers.as_ref() };
    // AudioBufferList has a variable-length trailing array, supplied by AVPlayer.
    let buffers =
        unsafe { std::slice::from_raw_parts(list.mBuffers.as_ptr(), list.mNumberBuffers as usize) };
    context
        .capture
        .push(unsafe { *frames_out.as_ref() }.max(0) as usize, |frame| {
            stereo_frame(buffers, &context.format, frame)
        });
}

fn supported(format: &AudioStreamBasicDescription) -> bool {
    let flags = format.mFormatFlags;
    let planar = flags & kAudioFormatFlagIsNonInterleaved != 0;
    let channels = if planar { 1 } else { format.mChannelsPerFrame };
    format.mFormatID == kAudioFormatLinearPCM
        && format.mSampleRate.is_finite()
        && format.mSampleRate > 0.0
        && format.mChannelsPerFrame > 0
        && flags & kAudioFormatFlagIsBigEndian == 0
        && flags & kAudioFormatFlagIsPacked != 0
        && format.mBytesPerFrame == channels * (format.mBitsPerChannel / 8)
        && ((flags & kAudioFormatFlagIsFloat != 0 && matches!(format.mBitsPerChannel, 32 | 64))
            || (flags & kAudioFormatFlagIsSignedInteger != 0
                && matches!(format.mBitsPerChannel, 16 | 32)))
}

fn stereo_frame(
    buffers: &[AudioBuffer],
    format: &AudioStreamBasicDescription,
    frame: usize,
) -> [f32; 2] {
    let mut stereo = [0.0; 2];
    let mut channels = 0;
    for buffer in buffers {
        for channel in 0..buffer.mNumberChannels as usize {
            if channels == 2 {
                return stereo;
            }
            let bytes = (format.mBitsPerChannel / 8) as usize;
            let offset = (frame * buffer.mNumberChannels as usize + channel) * bytes;
            if !buffer.mData.is_null() && offset + bytes <= buffer.mDataByteSize as usize {
                let data = unsafe { buffer.mData.cast::<u8>().add(offset) };
                // Bounds and representation are checked above and in prepare.
                stereo[channels] = unsafe {
                    if format.mFormatFlags & kAudioFormatFlagIsFloat != 0 {
                        if bytes == 4 {
                            data.cast::<f32>().read_unaligned()
                        } else {
                            data.cast::<f64>().read_unaligned() as f32
                        }
                    } else if bytes == 2 {
                        data.cast::<i16>().read_unaligned() as f32 / 32768.0
                    } else {
                        data.cast::<i32>().read_unaligned() as f32 / 2147483648.0
                    }
                };
            }
            channels += 1;
        }
    }
    if channels == 1 {
        stereo[1] = stereo[0];
    }
    stereo
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_interleaved_and_planar_stereo_without_modifying_pcm() {
        let mut data = [0.25_f32, -0.25, 0.5, -0.5];
        let format = AudioStreamBasicDescription {
            mSampleRate: 48_000.0,
            mFormatID: kAudioFormatLinearPCM,
            mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked,
            mChannelsPerFrame: 2,
            mBitsPerChannel: 32,
            mBytesPerFrame: 8,
            ..empty_format()
        };
        assert!(supported(&format));
        let buffer = AudioBuffer {
            mNumberChannels: 2,
            mDataByteSize: 16,
            mData: data.as_mut_ptr().cast(),
        };
        assert_eq!(stereo_frame(&[buffer], &format, 1), [0.5, -0.5]);
        assert_eq!(data, [0.25, -0.25, 0.5, -0.5]);
        let planar = AudioStreamBasicDescription {
            mFormatFlags: format.mFormatFlags | kAudioFormatFlagIsNonInterleaved,
            mBytesPerFrame: 4,
            ..format
        };
        assert!(supported(&planar));
        let buffers = [
            AudioBuffer {
                mNumberChannels: 1,
                mDataByteSize: 8,
                mData: data.as_mut_ptr().cast(),
            },
            AudioBuffer {
                mNumberChannels: 1,
                mDataByteSize: 8,
                mData: unsafe { data.as_mut_ptr().add(2).cast() },
            },
        ];
        assert_eq!(stereo_frame(&buffers, &planar, 0), [0.25, 0.5]);
        assert_eq!(stereo_frame(&buffers, &planar, 20), [0.0, 0.0]);
    }
}

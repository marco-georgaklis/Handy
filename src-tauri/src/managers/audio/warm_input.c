// Core Audio timestamps identify the *first captured sample* even when several
// converted input buffers reach this callback together.
#include <AudioToolbox/AudioToolbox.h>
#include <CoreAudio/HostTime.h>
#include <CoreFoundation/CoreFoundation.h>
#include <stdint.h>

extern void handy_warm_block(void *context, const float *samples,
                             uint32_t frames, uint64_t first_sample_host_ns);
extern void handy_warm_failed(void *context);

uint64_t handy_warm_now_ns(void) {
    return AudioConvertHostTimeToNanos(AudioGetCurrentHostTime());
}

static void input_ready(void *context, AudioQueueRef queue, AudioQueueBufferRef buffer,
                        const AudioTimeStamp *time, UInt32 packets,
                        const AudioStreamPacketDescription *descriptions) {
    (void)packets;
    (void)descriptions;
    const uint64_t now = AudioGetCurrentHostTime();
    if ((time->mFlags & kAudioTimeStampHostTimeValid) && time->mHostTime <= now) {
        handy_warm_block(context, buffer->mAudioData, buffer->mAudioDataByteSize / sizeof(float),
                         AudioConvertHostTimeToNanos(time->mHostTime));
    } else {
        handy_warm_failed(context);
    }
    if (AudioQueueEnqueueBuffer(queue, buffer, 0, NULL) != noErr)
        handy_warm_failed(context);
}

static OSStatus select_device(AudioQueueRef queue, const char *uid) {
    if (!uid) return noErr;
    CFStringRef device = CFStringCreateWithCString(NULL, uid, kCFStringEncodingUTF8);
    if (!device) return kAudioQueueErr_InvalidParameter;
    OSStatus status = AudioQueueSetProperty(queue, kAudioQueueProperty_CurrentDevice,
                                            &device, sizeof(device));
    CFRelease(device);
    return status;
}

AudioQueueRef handy_warm_open(void *context, UInt32 frames, const char *uid, OSStatus *status) {
    AudioStreamBasicDescription speech = {
        .mSampleRate = 16000, .mFormatID = kAudioFormatLinearPCM,
        .mFormatFlags = kAudioFormatFlagsNativeFloatPacked, .mBytesPerPacket = sizeof(float),
        .mFramesPerPacket = 1, .mBytesPerFrame = sizeof(float), .mChannelsPerFrame = 1,
        .mBitsPerChannel = 32,
    };
    AudioQueueRef queue = NULL;
    *status = AudioQueueNewInput(&speech, input_ready, context, NULL, NULL, 0, &queue);
    if (*status != noErr) return NULL;
    *status = select_device(queue, uid);
    for (int i = 0; *status == noErr && i < 6; ++i) {
        AudioQueueBufferRef buffer;
        *status = AudioQueueAllocateBuffer(queue, frames * sizeof(float), &buffer);
        if (*status == noErr) *status = AudioQueueEnqueueBuffer(queue, buffer, 0, NULL);
    }
    if (*status == noErr) *status = AudioQueueStart(queue, NULL);
    if (*status == noErr) return queue;
    AudioQueueDispose(queue, true);
    return NULL;
}

void handy_warm_close(AudioQueueRef queue) {
    AudioQueueDispose(queue, true);
}

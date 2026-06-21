/*
 * SassyTalkie - libopus JNI bridge
 *
 * Maps to com.sassyconsulting.sassytalkie.audio.OpusEncoder native methods.
 *
 * Build: see CMakeLists.txt. Requires libopus headers and a per-ABI .so or
 * static lib for libopus. Recommended: pull opus 1.4 via Prefab or vendor
 * prebuilts under src/main/jniLibs/<abi>/libopus.so with headers in
 * src/main/cpp/include/opus/.
 */
#include <jni.h>
#include <opus.h>
#include <stdint.h>
#include <string.h>

#define MAX_OPUS_PACKET 4000
#define LOG_TAG "sassytalkie_opus"

JNIEXPORT jlong JNICALL
Java_com_sassyconsulting_sassytalkie_audio_OpusEncoder_nativeCreate(
        JNIEnv *env, jobject thiz, jint sr, jint ch) {
    int err = 0;
    OpusEncoder *enc = opus_encoder_create(sr, ch, OPUS_APPLICATION_VOIP, &err);
    if (err != OPUS_OK || !enc) return 0;
    return (jlong)(intptr_t)enc;
}

JNIEXPORT void JNICALL
Java_com_sassyconsulting_sassytalkie_audio_OpusEncoder_nativeDestroy(
        JNIEnv *env, jobject thiz, jlong h) {
    if (h) opus_encoder_destroy((OpusEncoder*)(intptr_t)h);
}

JNIEXPORT jbyteArray JNICALL
Java_com_sassyconsulting_sassytalkie_audio_OpusEncoder_nativeEncode(
        JNIEnv *env, jobject thiz, jlong h, jshortArray pcm, jint frameSize) {
    OpusEncoder *enc = (OpusEncoder*)(intptr_t)h;
    if (!enc) return NULL;

    jshort *pcmPtr = (*env)->GetShortArrayElements(env, pcm, NULL);
    if (!pcmPtr) return NULL;

    unsigned char buf[MAX_OPUS_PACKET];
    int nbytes = opus_encode(enc, (opus_int16*)pcmPtr, frameSize, buf, MAX_OPUS_PACKET);
    (*env)->ReleaseShortArrayElements(env, pcm, pcmPtr, JNI_ABORT);

    if (nbytes < 0) return NULL;
    jbyteArray out = (*env)->NewByteArray(env, nbytes);
    if (!out) return NULL;
    (*env)->SetByteArrayRegion(env, out, 0, nbytes, (const jbyte*)buf);
    return out;
}

#define CTL_INT(NAME, OPUSCTL) \
JNIEXPORT void JNICALL \
Java_com_sassyconsulting_sassytalkie_audio_OpusEncoder_##NAME( \
        JNIEnv *env, jobject thiz, jlong h, jint v) { \
    OpusEncoder *enc = (OpusEncoder*)(intptr_t)h; \
    if (enc) opus_encoder_ctl(enc, OPUSCTL(v)); \
}

#define CTL_BOOL(NAME, OPUSCTL) \
JNIEXPORT void JNICALL \
Java_com_sassyconsulting_sassytalkie_audio_OpusEncoder_##NAME( \
        JNIEnv *env, jobject thiz, jlong h, jboolean v) { \
    OpusEncoder *enc = (OpusEncoder*)(intptr_t)h; \
    if (enc) opus_encoder_ctl(enc, OPUSCTL(v ? 1 : 0)); \
}

CTL_INT(nativeSetBitrate,         OPUS_SET_BITRATE)
CTL_INT(nativeSetComplexity,      OPUS_SET_COMPLEXITY)
CTL_INT(nativeSetApplication,     OPUS_SET_APPLICATION)
CTL_INT(nativeSetSignal,          OPUS_SET_SIGNAL)
CTL_BOOL(nativeSetVbr,            OPUS_SET_VBR)
CTL_BOOL(nativeSetDtx,            OPUS_SET_DTX)
CTL_BOOL(nativeSetInbandFec,      OPUS_SET_INBAND_FEC)
CTL_INT(nativeSetPacketLossPerc,  OPUS_SET_PACKET_LOSS_PERC)

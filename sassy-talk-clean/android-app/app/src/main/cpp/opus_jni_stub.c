/*
 * SassyTalkie - libopus stub
 *
 * Built when libopus prebuilts are NOT present in the tree. The library still
 * loads successfully (so OpusEncoder.nativeAvailable = true) but every native
 * method returns a failure sentinel. OpusEncoder.<init> will throw the moment
 * any caller actually tries to construct one — so the rest of the audio
 * pipeline (mic gate, calibration, effects, router) still builds and runs.
 *
 * To enable the real encoder: drop opus headers under src/main/cpp/include/opus/
 * and per-ABI libopus.so files under src/main/jniLibs/<abi>/, then rebuild.
 * CMakeLists.txt will pick them up automatically.
 */
#include <jni.h>
#include <stdint.h>

JNIEXPORT jlong JNICALL
Java_com_sassyconsulting_sassytalkie_audio_OpusEncoder_nativeCreate(
        JNIEnv *env, jobject thiz, jint sr, jint ch) {
    (void)env; (void)thiz; (void)sr; (void)ch;
    return 0;
}

JNIEXPORT void JNICALL
Java_com_sassyconsulting_sassytalkie_audio_OpusEncoder_nativeDestroy(
        JNIEnv *env, jobject thiz, jlong h) {
    (void)env; (void)thiz; (void)h;
}

JNIEXPORT jbyteArray JNICALL
Java_com_sassyconsulting_sassytalkie_audio_OpusEncoder_nativeEncode(
        JNIEnv *env, jobject thiz, jlong h, jshortArray pcm, jint frameSize) {
    (void)env; (void)thiz; (void)h; (void)pcm; (void)frameSize;
    return NULL;
}

#define CTL_INT_STUB(NAME) \
JNIEXPORT void JNICALL \
Java_com_sassyconsulting_sassytalkie_audio_OpusEncoder_##NAME( \
        JNIEnv *env, jobject thiz, jlong h, jint v) { \
    (void)env; (void)thiz; (void)h; (void)v; \
}

#define CTL_BOOL_STUB(NAME) \
JNIEXPORT void JNICALL \
Java_com_sassyconsulting_sassytalkie_audio_OpusEncoder_##NAME( \
        JNIEnv *env, jobject thiz, jlong h, jboolean v) { \
    (void)env; (void)thiz; (void)h; (void)v; \
}

CTL_INT_STUB(nativeSetBitrate)
CTL_INT_STUB(nativeSetComplexity)
CTL_INT_STUB(nativeSetApplication)
CTL_INT_STUB(nativeSetSignal)
CTL_BOOL_STUB(nativeSetVbr)
CTL_BOOL_STUB(nativeSetDtx)
CTL_BOOL_STUB(nativeSetInbandFec)
CTL_INT_STUB(nativeSetPacketLossPerc)

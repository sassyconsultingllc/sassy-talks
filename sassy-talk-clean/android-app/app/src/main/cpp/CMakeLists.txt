cmake_minimum_required(VERSION 3.22.1)
project(sassytalkie_opus C)

# libopus integration. We support three modes, selected automatically based on
# what's present in the tree:
#
#   1) Vendor prebuilts:
#        src/main/cpp/include/opus/opus.h
#        src/main/jniLibs/<abi>/libopus.so
#      The JNI bridge links against the prebuilt and is fully functional.
#
#   2) Headers without prebuilt: not supported. Drop both, or neither.
#
#   3) Neither present: we build a stub libsassytalkie_opus.so with no JNI
#      bindings. Kotlin's System.loadLibrary still succeeds, but OpusEncoder
#      reports nativeAvailable=false and constructing one throws. The pipeline
#      degrades gracefully (calibration + gate work, encoder doesn't), and the
#      rest of the app builds. Drop in libopus prebuilts and rebuild to enable
#      the encoder path.

set(CMAKE_C_STANDARD 99)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

set(OPUS_HEADER ${CMAKE_SOURCE_DIR}/include/opus/opus.h)
set(OPUS_SO    ${CMAKE_SOURCE_DIR}/../jniLibs/${ANDROID_ABI}/libopus.so)

find_library(log-lib log)

# 16 KB page-size alignment (Google Play requirement for Android 15+ / targetSdk 35).
# NDK r25 links LOAD segments at 4 KB by default; force 16 KB so this .so loads
# on 16 KB-page devices. Applies to every target defined below.
add_link_options("-Wl,-z,max-page-size=16384")

if(EXISTS ${OPUS_HEADER} AND EXISTS ${OPUS_SO})
    message(STATUS "sassytalkie_opus: libopus found for ${ANDROID_ABI}, building JNI bridge")

    add_library(opus_prebuilt SHARED IMPORTED)
    set_target_properties(opus_prebuilt PROPERTIES
        IMPORTED_LOCATION ${OPUS_SO})

    add_library(sassytalkie_opus SHARED opus_jni.c)

    target_include_directories(sassytalkie_opus PRIVATE
        ${CMAKE_SOURCE_DIR}/include
        ${CMAKE_SOURCE_DIR}/include/opus)

    target_link_libraries(sassytalkie_opus
        opus_prebuilt
        ${log-lib})
else()
    message(WARNING
        "sassytalkie_opus: libopus not vendored (looked for ${OPUS_HEADER} and ${OPUS_SO}). "
        "Building a stub library so the project still compiles. The Opus encoder "
        "will report nativeAvailable=false at runtime — drop libopus.so + headers in "
        "to enable it.")

    add_library(sassytalkie_opus SHARED opus_jni_stub.c)
    target_link_libraries(sassytalkie_opus ${log-lib})
endif()

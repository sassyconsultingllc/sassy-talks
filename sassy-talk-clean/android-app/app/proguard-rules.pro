# Keep native methods
-keepclasseswithmembernames class * {
    native <methods>;
}

# Keep SassyTalkNative class
-keep class com.sassyconsulting.sassytalkie.SassyTalkNative { *; }

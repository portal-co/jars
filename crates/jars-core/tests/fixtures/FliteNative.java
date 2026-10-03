public class FliteNative {
    public static void main(String[] args) {
        register(null);
    }

    static com.sun.jna.Pointer register(
        com.mojang.text2speech.NarratorLinux.FliteLibrary.CmuUsKal16 library
    ) {
        return library.register_cmu_us_kal16("kal16");
    }
}

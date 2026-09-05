package app;

public class UnsupportedPlatform {
    public static void run() {
        java.util.stream.Stream.of("x");
    }
}

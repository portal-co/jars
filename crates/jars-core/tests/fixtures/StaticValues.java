public class StaticValues {
    public static final int BASE = 32;
    public static int value = 10;

    static {
        value = value + BASE;
    }

    public static int next() {
        value = value + 1;
        return value;
    }
}

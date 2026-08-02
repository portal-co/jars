public class ThrowNull {
    public static int value() {
        try {
            throw null;
        } catch (NullPointerException error) {
            return 42;
        }
    }

    public static void main(String[] args) {
        System.out.println(value());
    }
}

public class TypedLongCatch {
    public static long value() {
        try {
            return 84L / 0L;
        } catch (ArithmeticException error) {
            return 42L;
        }
    }

    public static void main(String[] args) {
        System.out.println(value());
    }
}

public class FinallyRethrows {
    public static int value() {
        try {
            try {
                return 1 / 0;
            } finally {
                int ignored = 1;
            }
        } catch (ArithmeticException error) {
            return 42;
        }
    }

    public static void main(String[] args) {
        System.out.println(value());
    }
}

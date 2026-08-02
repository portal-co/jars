public class CatchArithmetic {
    public static int value() {
        try {
            return 1 / 0;
        } catch (ArithmeticException error) {
            return 42;
        }
    }

    public static void main(String[] args) {
        System.out.println(value());
    }
}

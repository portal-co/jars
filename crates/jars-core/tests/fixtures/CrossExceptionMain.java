public class CrossExceptionMain {
    public static int value() {
        try {
            return ExceptionHelper.fail();
        } catch (ArithmeticException error) {
            return 42;
        }
    }

    public static void main(String[] args) {
        System.out.println(value());
    }
}

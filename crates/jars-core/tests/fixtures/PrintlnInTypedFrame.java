public class PrintlnInTypedFrame {
    public static void main(String[] args) {
        try {
            System.out.println(1 / 0);
        } catch (ArithmeticException error) {
            System.out.println(Math.min(42, 100));
        }
    }
}

public class InstanceTypedCatch {
    public InstanceTypedCatch() {}

    public int divide(int left, int right) {
        try {
            return left / right;
        } catch (ArithmeticException error) {
            return 42;
        }
    }

    public static void main(String[] args) {
        System.out.println(new InstanceTypedCatch().divide(1, 0));
    }
}

public class FramedArrayRefs {
    public static int outerLength(int[][] values) {
        try {
            return 1 / 0;
        } catch (ArithmeticException error) {
            return values.length;
        }
    }

    public static int createdLength() {
        try {
            return 1 / 0;
        } catch (ArithmeticException error) {
            return new int[42].length;
        }
    }

    public static void main(String[] args) {
        System.out.println(outerLength(new int[42][1]));
        System.out.println(createdLength());
    }
}

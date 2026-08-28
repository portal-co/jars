public class FramedReferenceControl {
    public static int sameBox(PublicBox left, PublicBox right) {
        try {
            return 1 / 0;
        } catch (ArithmeticException error) {
            if (left == null) {
                return -1;
            }
            if (left == right) {
                return left.value;
            }
            return 0;
        }
    }

    public static int sameArray(int[] left, int[] right) {
        try {
            return 1 / 0;
        } catch (ArithmeticException error) {
            if (left == right) {
                return left.length;
            }
            return 0;
        }
    }

    public static void main(String[] args) {
        PublicBox box = new PublicBox(42);
        int[] values = new int[42];
        System.out.println(sameBox(box, box));
        System.out.println(sameBox(null, box));
        System.out.println(sameArray(values, values));
    }
}

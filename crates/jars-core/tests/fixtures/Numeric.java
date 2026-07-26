public class Numeric {
    public static long longMath(long left, long right) {
        long product = left * right;
        return product / right + left - right;
    }

    public static float floatMath(float value) {
        return value / 2.0f + 0.5f;
    }

    public static double doubleMath(double value) {
        return value * 2.0 - 0.25;
    }

    public static void main(String[] args) {
        System.out.println(longMath(20L, 2L));
        System.out.println(floatMath(4.0f));
        System.out.println(doubleMath(21.125));
    }
}

public class Arithmetic {
    public static int sumTo(int limit) {
        int total = 0;
        int current = 1;
        while (current <= limit) {
            total += current;
            current++;
        }
        return total;
    }

    public static int bits(int value) {
        return (value << 2) ^ (value >>> 1);
    }

    public static void main(String[] args) {
        System.out.println(sumTo(9) + bits(4));
    }
}

public class Accumulator {
    private int total;

    public Accumulator(int initial) {
        total = initial;
    }

    public void add(int value) {
        total = total + value;
    }

    public int get() {
        return total;
    }

    public static void main(String[] args) {
        Accumulator accumulator = new Accumulator(10);
        accumulator.add(32);
        System.out.println(accumulator.get());
    }
}

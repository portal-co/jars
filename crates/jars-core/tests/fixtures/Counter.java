public class Counter {
    private int total;

    public Counter(int initial) {
        total = initial;
    }

    public int add(int value) {
        total = total + value;
        return total;
    }
}

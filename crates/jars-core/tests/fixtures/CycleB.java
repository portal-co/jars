public class CycleB {
    public CycleA source;

    public CycleB() {
    }

    public int fromA() {
        return source.finish();
    }
}

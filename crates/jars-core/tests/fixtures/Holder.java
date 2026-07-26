public class Holder {
    private Box box;

    public Holder(Box box) {
        this.box = box;
    }

    public Box get() {
        return box;
    }

    public static Box identity(Box box) {
        return box;
    }
}

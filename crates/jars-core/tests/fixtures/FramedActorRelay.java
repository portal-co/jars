public class FramedActorRelay {
    private int offset;

    public FramedActorRelay() {
        offset = 1;
    }

    public int update(PublicBox box) {
        try {
            return 1 / 0;
        } catch (ArithmeticException error) {
            offset = offset + 1;
            box.value = box.value + offset;
            return box.read();
        }
    }

    public static void main(String[] args) {
        FramedActorRelay relay = new FramedActorRelay();
        System.out.println(relay.update(new PublicBox(40)));
    }
}

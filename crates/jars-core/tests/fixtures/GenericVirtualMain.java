public class GenericVirtualMain {
    public static void main(String[] args) {
        Relay relay = new Relay();
        PublicBox box = new PublicBox(42);
        System.out.println(relay.relay(box));
    }
}

public class ObjectReferencesMain {
    public static void main(String[] args) {
        Box box = new Box(42);
        Holder holder = new Holder(box);
        Box result = Holder.identity(holder.get());
        System.out.println(result.value());
    }
}

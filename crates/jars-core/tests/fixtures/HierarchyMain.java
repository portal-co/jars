public class HierarchyMain {
    public static void main(String[] args) {
        BaseValue value = new DerivedValue();
        System.out.println(value.value());
    }
}

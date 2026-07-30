public class StaticFieldsMain {
    public static void main(String[] args) {
        System.out.println(StaticValues.value);
        StaticValues.value = 1;
        System.out.println(StaticValues.next());
    }
}

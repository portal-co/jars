public class CrossFieldsMain {
    public static void main(String[] args) {
        PublicBox left = new PublicBox(10);
        PublicBox right = new PublicBox(32);
        right.value = left.value + right.value;
        System.out.println(right.value);
    }
}

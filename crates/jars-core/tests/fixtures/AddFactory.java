public class AddFactory {
    public int add(int left, int right) {
        return left + right;
    }

    public static void main(String[] args) {
        System.out.println(new AddFactory().add(20, 22));
    }
}

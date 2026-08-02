public class TypedClinit {
    static int result;

    static {
        int value = 40;
        if (value == 40) {
            result = value + 2;
        }
    }

    public static void main(String[] args) {
        System.out.println(result);
    }
}

public class Switches {
    public static int dense(int value) {
        switch (value) {
            case 1: return 40;
            case 2: return 42;
            default: return 0;
        }
    }

    public static int sparse(int value) {
        switch (value) {
            case 1: return 1;
            case 100: return 42;
            default: return 0;
        }
    }

    public static void main(String[] args) {
        System.out.println(dense(2));
        System.out.println(sparse(100));
    }
}

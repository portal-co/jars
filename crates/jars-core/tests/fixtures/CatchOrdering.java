public class CatchOrdering {
    public static int value() {
        try {
            return 1 / 0;
        } catch (NullPointerException error) {
            return 1;
        } catch (RuntimeException error) {
            return 42;
        }
    }

    public static void main(String[] args) {
        System.out.println(value());
    }
}

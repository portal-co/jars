public class FramedObjectOps {
    public static int updateAndRead(PublicBox box) {
        try {
            int ignored = 1 / 0;
            return ignored;
        } catch (ArithmeticException error) {
            box.value = box.value + 1;
            return box.read();
        }
    }

    public static void main(String[] args) {
        System.out.println(updateAndRead(new PublicBox(41)));
    }
}

public class CycleMain {
    public static void main(String[] args) {
        CycleB partner = new CycleB();
        CycleA source = new CycleA(partner);
        partner.source = source;
        System.out.println(source.start());
    }
}

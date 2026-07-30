public class CycleA {
    private CycleB partner;

    public CycleA(CycleB partner) {
        this.partner = partner;
    }

    public int start() {
        return partner.fromA();
    }

    public int finish() {
        return 42;
    }
}

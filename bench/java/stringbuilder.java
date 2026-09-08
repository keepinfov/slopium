// String building, incrementally: 50,000 records of "item " + number + '\n'
// appended to one StringBuilder, then the result measured.
//
// The checksum is the byte length of the result and the sum of its bytes.

public class stringbuilder {
    static final int RECORDS = 50000;

    public static void main(String[] args) {
        StringBuilder out = new StringBuilder();
        for (int i = 0; i < RECORDS; i++) {
            out.append("item ");
            out.append(i);
            out.append('\n');
        }
        byte[] bytes = out.toString().getBytes(java.nio.charset.StandardCharsets.US_ASCII);
        long sum = 0;
        for (byte b : bytes)
            sum += b & 0xFF;
        System.out.println(bytes.length);
        System.out.println(sum);
    }
}

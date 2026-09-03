// String building, incrementally: 50,000 records of "item " + number + '\n'
// appended to one growing String, then the result measured.
//
// The checksum is the byte length of the result and the sum of its bytes.

const RECORDS: i64 = 50000;

fn main() {
    let mut out = String::new();
    for i in 0..RECORDS {
        out.push_str("item ");
        out.push_str(&i.to_string());
        out.push('\n');
    }
    let mut sum: i64 = 0;
    for byte in out.bytes() {
        sum += byte as i64;
    }
    println!("{}", out.len());
    println!("{}", sum);
}

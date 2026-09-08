# String building, incrementally: 50,000 records of "item " + number + '\n'
# appended to a list of parts joined once, which is the idiomatic fast path
# in CPython.
#
# The checksum is the byte length of the result and the sum of its bytes.

RECORDS = 50000

parts = []
for i in range(RECORDS):
    parts.append("item ")
    parts.append(str(i))
    parts.append("\n")

text = "".join(parts)

print(len(text))
print(sum(map(ord, text)))

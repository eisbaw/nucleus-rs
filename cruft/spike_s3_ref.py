import struct
NW=4; PS=8; N=NW*PS  # 32
# Deterministic, varied, non-monotonic, mixed sign. Stays well inside i32.
new = [ (i*7) % 1000 - 500 for i in range(N) ]
old = [ (i*13) % 1000 - 500 for i in range(N) ]
# reference: phase1 per-partition max(|new-old|) folded into partials (init 0 = max-identity since abs>=0),
# phase2 tree-combine to scalar maxdiff.
partials=[0]*NW
for w in range(NW):
    for i in range(PS):
        idx=w*PS+i
        d=abs(new[idx]-old[idx])
        partials[w]=max(partials[w], d)
half1=max(partials[0],partials[1])
half2=max(partials[2],partials[3])
maxdiff=max(half1,half2)
# direct (order-free) check
direct=max(abs(new[i]-old[i]) for i in range(N))
assert maxdiff==direct, (maxdiff, direct)
print("partials",partials,"maxdiff",maxdiff)
# write input.bin = new||old concatenated row-major (NW*PS each)
buf=bytearray()
for v in new: buf+=struct.pack('<i',v)
for v in old: buf+=struct.pack('<i',v)
open('/home/mpedersen/topics/mark_thesis/cruft/s3_input.bin','wb').write(buf)
open('/home/mpedersen/topics/mark_thesis/cruft/s3_reference.bin','wb').write(struct.pack('<i',maxdiff))
print("input bytes", len(buf), "ref", maxdiff)

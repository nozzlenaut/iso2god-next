// One work item hashes exactly one 4096-byte GOD data block.
// Partial final blocks are hashed on the CPU.
inline uint rol(uint x, uint n) { return rotate(x, n); }
__kernel void sha1_blocks(__global const uchar *input, __global uchar *output) {
    size_t id = get_global_id(0);
    __global const uchar *p = input + id * 4096;
    uint h0=0x67452301U, h1=0xefcdab89U, h2=0x98badcfeU,
         h3=0x10325476U, h4=0xc3d2e1f0U;
    // 64 data chunks + one full padding chunk (4096 * 8 bits).
    for (uint chunk=0; chunk<65; ++chunk) {
        uint w[16];
        #pragma unroll 16
        for (uint j=0; j<16; ++j) {
            if (chunk==64) {
                w[j] = j==0 ? 0x80000000U : (j==15 ? 32768U : 0U);
            } else {
                size_t k=(size_t)chunk*64+j*4;
                w[j]=((uint)p[k]<<24)|((uint)p[k+1]<<16)|((uint)p[k+2]<<8)|(uint)p[k+3];
            }
        }
        uint a=h0,b=h1,c=h2,d=h3,e=h4;
        #pragma unroll 80
        for (uint t=0;t<80;++t) {
            if(t>=16) w[t&15]=rol(w[(t-3)&15]^w[(t-8)&15]^w[(t-14)&15]^w[t&15],1U);
            uint f,k;
            if(t<20){ f=(b&c)|(~b&d);k=0x5a827999U; }
            else if(t<40){ f=b^c^d;k=0x6ed9eba1U; }
            else if(t<60){ f=(b&c)|(b&d)|(c&d);k=0x8f1bbcdcU; }
            else { f=b^c^d;k=0xca62c1d6U; }
            uint tmp=rol(a,5U)+f+e+k+w[t&15];
            e=d;d=c;c=rol(b,30U);b=a;a=tmp;
        }
        h0+=a;h1+=b;h2+=c;h3+=d;h4+=e;
    }
    uint h[5]={h0,h1,h2,h3,h4};
    for(uint j=0;j<5;++j)
        for(uint k=0;k<4;++k)
            output[id*20+j*4+k]=(uchar)(h[j]>>(24-8*k));
}

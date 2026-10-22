# Generates blue_noise_128.r8: python3 blue_noise_128.py blue_noise_128.r8 (about 10 s).
# Void-and-cluster (Ulichney 1993) blue noise, 128 x 128, toroidal Gaussian sigma 1.5,
# kernel radius 7, seeded initial pattern. Writes ranks scaled to 0..255, row-major.
import math, random, sys
N=128; S=1.5; R=7
rng=random.Random(20261021)
K=[]
for dy in range(-R,R+1):
    for dx in range(-R,R+1):
        K.append((dx,dy,math.exp(-(dx*dx+dy*dy)/(2*S*S))))
def add(E,i,sgn):
    x,y=i%N,i//N
    for dx,dy,w in K:
        E[((y+dy)%N)*N+(x+dx)%N]+=sgn*w
P=[0]*(N*N); E=[0.0]*(N*N)
ones=rng.sample(range(N*N), N*N//10)
for i in ones: P[i]=1; add(E,i,1)
def tightest(E,P):  # max energy among ones
    best=-1e18; bi=-1
    for i in range(N*N):
        if P[i] and E[i]>best: best=E[i]; bi=i
    return bi
def largest_void(E,P):
    best=1e18; bi=-1
    for i in range(N*N):
        if not P[i] and E[i]<best: best=E[i]; bi=i
    return bi
# relax initial pattern
while True:
    c=tightest(E,P); P[c]=0; add(E,c,-1)
    v=largest_void(E,P)
    if v==c: P[c]=1; add(E,c,1); break
    P[v]=1; add(E,v,1)
rank=[0]*(N*N)
proto=P[:]; Ep=E[:]
ones_n=sum(P)
# phase 1: remove ones
r=ones_n-1
while r>=0:
    c=tightest(Ep,proto); proto[c]=0; add(Ep,c,-1); rank[c]=r; r-=1
# phase 2/3: add zeros
proto=P[:]; Ep=E[:]
r=ones_n
while r<N*N:
    v=largest_void(Ep,proto); proto[v]=1; add(Ep,v,1); rank[v]=r; r+=1
    if r%2048==0: print(r,file=sys.stderr)
out=bytes(int(rk*256//(N*N)) for rk in rank)
open(sys.argv[1],'wb').write(out)

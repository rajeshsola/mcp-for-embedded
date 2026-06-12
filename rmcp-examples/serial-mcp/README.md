```
sudp apt install socat
socat -d -d pty,raw,echo=0 pty,raw,echo=0
stty -F /dev/ttyS0 9600 cs8 -cstopb -parenb

sudo cat /dev/pts/6
echo "how r u" | sudo tee /dev/pts/7
```

#!/bin/sh
# Generate the test site: ~2.5 MB of noise JPEGs plus a 2-minute seekable MP4.
set -e
cd "$(dirname "$0")/testsite"
for i in 1 2 3 4 5 6 7 8; do
  ffmpeg -loglevel error -y -f lavfi -i "nullsrc=s=1000x700,geq=random(1)*255:128:128" -frames:v 1 -q:v 9 img$i.jpg
done
ffmpeg -loglevel error -y -f lavfi -i testsrc2=s=1280x720:r=30:d=120 -f lavfi -i sine=d=120 \
  -c:v libx264 -b:v 1500k -g 60 -c:a aac -movflags +faststart video.mp4

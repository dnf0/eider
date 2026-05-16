FROM rust:1.80
RUN apt-get update && apt-get install -y clang cmake
WORKDIR /app
CMD ["cargo", "build", "--release"]

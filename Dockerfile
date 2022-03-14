FROM ubuntu:latest
WORKDIR /root/

ENV TZ=America/New_York
RUN ln -snf /usr/share/zoneinfo/$TZ /etc/localtime && echo $TZ > /etc/timezone

RUN apt-get update && apt-get install -y \
  wget \
  build-essential \
  curl \
  git-all \
  libgmp-dev \
  libsecp256k1-dev \
  libsodium-dev \
  libssl-dev \
  libudev-dev \
  pkg-config \
  python3 \
  python3-pip \
  software-properties-common

RUN apt-get update

RUN curl https://sh.rustup.rs -sSf | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

RUN pip3 install pytezos==3.2.11

# Define the branch here to prevent caching old versions of the repository
ARG BRANCH=main
RUN git clone https://github.com/boltlabs-inc/zeekoe.git 
WORKDIR /root/zeekoe

RUN git checkout $BRANCH
RUN git submodule update --init --recursive
RUN ./dev/generate-certificates; CARGO_NET_GIT_FETCH_WITH_CLI=true cargo build --features "allow_explicit_certificate_trust allow_custom_self_delay"

RUN wget https://github.com/serokell/tezos-packaging/releases/latest/download/tezos-client
RUN chmod +x tezos-client
RUN mkdir -p $HOME/.local/bin
RUN mv tezos-client /usr/local/bin

CMD bash 

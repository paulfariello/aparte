set -ex

main() {
    if [ $TRAVIS_OS_NAME = linux ]; then
        sort=sort
    else
        sort=gsort  # for `sort --sort-version`, from brew's coreutils.
    fi

    # This fetches latest stable release
    local ver=$(git ls-remote --tags --refs --exit-code https://github.com/japaric/cross \
                       | cut -d/ -f3 \
                       | grep -E '^v[0.1.0-9.]+$' \
                       | $sort --version-sort \
                       | tail -n1 \
                       | sed 's/^v//')

    local img="aparte/cross:$TARGET-$ver"

    cat > Cross.toml <<EOF
[target.$TARGET]
image = "$img"
EOF


    docker build -t $img - <<EOF
FROM rustembedded/cross:$TARGET-$ver

RUN rustup component add rustfmt-preview
RUN apt-get update && \
    apt-get install --assume-yes libssl-dev
EOF
}

main

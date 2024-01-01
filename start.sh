#!/bin/sh

export CD_CONFIG_PATH=/usr/src/chunkdrive/config.yml
yq -i ".buckets.drive.source.url = \"$CHUNKDRIVE_DISCORD_WEBHOOK\"" $CD_CONFIG_PATH
yq -i ".buckets.drive.encryption.key = \"$CHUNKDRIVE_ENCRYPTION_KEY\"" $CD_CONFIG_PATH

cd /usr/src/chunkdrive/release
./chunkdrive

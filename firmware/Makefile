# One command per board. Boards and their Rust targets:
#
#   freenove-35    Freenove ESP32 Display 3.5" (ESP32)
#   crowpanel-579  Elecrow CrowPanel 5.79" e-paper (ESP32-S3)
#
# make build-<board>                  release image
# make clippy-<board>                 lints for one board
# make flash-<board> [PORT=...]       USB flash (erases the OTA state)
# make image-<board>                  target/<board>.bin, for /api/ota
# make full-<board>                   target/<board>-full.bin: bootloader,
#                                     partitions and app, written at 0x0
# make ota-<board> HOST=... [TOKEN=...]  update over WiFi
# make all                            build and lint every board
#
# Needs the ESP Rust toolchain (`espup install`, then `. ~/export-esp.sh`).

BOARDS := freenove-35 crowpanel-579
TARGET_freenove-35 := xtensa-esp32-espidf
CHIP_freenove-35 := esp32
FLASH_freenove-35 := 4mb
TARGET_crowpanel-579 := xtensa-esp32s3-espidf
CHIP_crowpanel-579 := esp32s3
FLASH_crowpanel-579 := 8mb

PORT ?=
TOKEN ?=
cargo = ESP_IDF_SYS_ROOT_CRATE=herald-$(1) cargo $(2) --release -p herald-$(1) --target $(TARGET_$(1))
elf = target/$(TARGET_$(1))/release/herald-$(1)

.PHONY: all $(addprefix build-,$(BOARDS)) $(addprefix clippy-,$(BOARDS)) \
	$(addprefix flash-,$(BOARDS)) $(addprefix image-,$(BOARDS)) $(addprefix full-,$(BOARDS)) \
	$(addprefix ota-,$(BOARDS))

all: $(addprefix clippy-,$(BOARDS)) $(addprefix build-,$(BOARDS))

$(addprefix build-,$(BOARDS)): build-%:
	$(call cargo,$*,build)

$(addprefix clippy-,$(BOARDS)): clippy-%:
	$(call cargo,$*,clippy) -- -D warnings

$(addprefix flash-,$(BOARDS)): flash-%: build-%
	espflash flash $(if $(PORT),-p $(PORT)) --partition-table boards/$*/partitions.csv \
		--erase-parts otadata $(call elf,$*)

$(addprefix image-,$(BOARDS)): image-%: build-%
	espflash save-image --chip $(CHIP_$*) $(call elf,$*) target/$*.bin

# The gap up to the app is padded, so writing this at 0x0 also blanks the
# settings and the OTA state: a fresh install that starts in the first slot.
$(addprefix full-,$(BOARDS)): full-%: build-%
	espflash save-image --chip $(CHIP_$*) --flash-size $(FLASH_$*) --merge --skip-padding \
		--partition-table boards/$*/partitions.csv $(call elf,$*) target/$*-full.bin

$(addprefix ota-,$(BOARDS)): ota-%: image-%
	@test -n "$(HOST)" || (echo "HOST=<address> is required" && exit 1)
	@curl -sS --fail-with-body -H 'Content-Type: application/octet-stream' \
		$(if $(TOKEN),-H 'X-Herald-Token: $(TOKEN)') \
		--data-binary @target/$*.bin http://$(HOST)/api/ota
	@echo

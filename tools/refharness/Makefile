# Build of the reference harness. Normally invoked through build.sh, which
# sets ROOT, OUT and TC. Upstream C sources are compiled in place; upstream
# C++ units are copied byte-for-byte into $(OUT)/src first so that their
# quoted #includes do not pick up the sibling upstream headers that the
# harness replaces (clang always searches the includer's directory first).

HERE   := $(patsubst %/,%,$(dir $(abspath $(lastword $(MAKEFILE_LIST)))))
ROOT   ?= $(abspath $(HERE)/../..)
OUT    ?= $(ROOT)/.ref/refharness
TC     ?= $(ROOT)/.ref/toolchain/llvm-mingw-20261117-ucrt-ubuntu-22.04-x86_64/bin
UP     := $(ROOT)/upstream/src
CC     := $(TC)/x86_64-w64-mingw32-clang
CXX    := $(TC)/x86_64-w64-mingw32-clang++
PYTHON ?= python3

GEN := $(OUT)/gen
SRC := $(OUT)/src
OBJ := $(OUT)/obj

# include order: generated headers, harness app shims, VCL shims, upstream
INCLUDES := -I$(GEN) -I$(HERE)/shim/app -I$(HERE)/shim/vcl -I$(HERE)/shim \
  -I$(HERE)/shim/randpool \
  -I$(UP)/crypto/blake2 -I$(UP)/zxcvbn -I$(UP)/miniz -I$(UP)/util \
  -I$(UP)/random -I$(UP)/passw -I$(UP)/minilzo -I$(UP)/lua -I$(UP)/crypto \
  -I$(UP)/crypto/polarssl -I$(UP)/main

# defines from PwTech.cbproj (Release: NDEBUG; LUA_INT_TYPE=1) plus the
# Windows/VCL environment C++Builder provides by default
DEFINES := -DNDEBUG -DLUA_INT_TYPE=1 -DUNICODE -D_UNICODE -D_WIN32_WINNT=0x0A00 \
  -DWINVER=0x0A00 -DNOMINMAX

WARN := -Wno-unknown-pragmas -Wno-ignored-attributes -Wno-microsoft-cast \
  -Wno-deprecated-declarations -Wno-unused-value -Wno-invalid-offsetof \
  -Wno-writable-strings -Wno-c++11-narrowing -Wno-microsoft-template \
  -Wno-ignored-pragma-optimize -Wno-pragma-pack -Wno-dangling-else \
  -Wno-switch -Wno-parentheses -Wno-deprecated-non-prototype

OPT := -O2 -fno-strict-aliasing -ffp-contract=off -fno-fast-math

# C++Builder's <string.h> makes _lrotl (stdlib.h) visible to chacha.c
CFLAGS   := $(OPT) $(DEFINES) $(INCLUDES) $(WARN) -include stdlib.h -MMD -MP
CXXFLAGS := $(OPT) $(DEFINES) $(INCLUDES) $(WARN) -std=c++17 -fdeclspec \
  -fms-extensions -include $(HERE)/shim/prelude.h -MMD -MP

# headers with __property that the compiled units include
PROP_HEADERS := passw/PasswGen.h passw/PasswDatabase.h util/StringFileStreamW.h \
  random/EntropyManager.h util/Scripting.h
GEN_HEADERS := $(addprefix $(GEN)/,$(notdir $(PROP_HEADERS)))

UP_C := crypto/chacha.c crypto/polarssl/aes.c crypto/polarssl/aesni.c \
  crypto/polarssl/base64.c crypto/polarssl/padlock.c crypto/polarssl/sha1.c \
  crypto/polarssl/sha256.c crypto/polarssl/sha512.c crypto/blake2/blake2b.c \
  minilzo/minilzo.c miniz/miniz.c passw/diceware8k.c \
  $(filter-out lua/lua.c lua/luac.c,$(patsubst $(UP)/%,%,$(wildcard $(UP)/lua/*.c)))

UP_CXX := crypto/CryptText.cpp crypto/CryptUtil.cpp passw/PasswGen.cpp \
  passw/PasswDatabase.cpp random/AESCtrPRNG.cpp random/RandomGenerator.cpp \
  random/FastPRNG.cpp random/RandomPool.cpp random/EntropyManager.cpp \
  util/MemUtil.cpp util/UnicodeUtil.cpp util/StringFileStreamW.cpp \
  util/hrtimer.cpp util/Scripting.cpp zxcvbn/zxcvbn.cpp

HARNESS_CXX := harness.cpp shim/vcl/vclshim.cpp shim/vcl/vclthread.cpp shim/dinkum_compat.cpp \
  shim/app/app_shim.cpp shim/randpool/randpool_switch.cpp \
  shim/randpool/randpool_real.cpp

C_OBJS   := $(addprefix $(OBJ)/up/,$(UP_C:.c=.o))
CXX_SRCS := $(addprefix $(SRC)/,$(notdir $(UP_CXX)))
CXX_OBJS := $(addprefix $(OBJ)/up/,$(notdir $(UP_CXX:.cpp=.o)))
H_OBJS   := $(addprefix $(OBJ)/h/,$(notdir $(HARNESS_CXX:.cpp=.o)))

EXE := $(OUT)/refharness.exe

.PHONY: all clean
all: $(EXE)

$(EXE): $(C_OBJS) $(CXX_OBJS) $(H_OBJS)
	$(CXX) -static -municode -o $@ $^ -lbcrypt -luser32 -lgdi32 -ladvapi32 -lkernel32

$(GEN)/%.h: | $(GEN)
	$(PYTHON) $(HERE)/gen_props.py $(filter %/$(notdir $@),$(addprefix $(UP)/,$(PROP_HEADERS))) $@

$(GEN)/PasswGen.h: $(UP)/passw/PasswGen.h $(HERE)/gen_props.py
$(GEN)/PasswDatabase.h: $(UP)/passw/PasswDatabase.h $(HERE)/gen_props.py
$(GEN)/StringFileStreamW.h: $(UP)/util/StringFileStreamW.h $(HERE)/gen_props.py
$(GEN)/EntropyManager.h: $(UP)/random/EntropyManager.h $(HERE)/gen_props.py
$(GEN)/Scripting.h: $(UP)/util/Scripting.h $(HERE)/gen_props.py

# byte-for-byte copies of the upstream C++ units
define COPY_RULE
$(SRC)/$(notdir $(1)): $(UP)/$(1) | $(SRC)
	cp -p $$< $$@
endef
$(foreach f,$(UP_CXX),$(eval $(call COPY_RULE,$(f))))

$(OBJ)/up/%.o: $(UP)/%.c
	@mkdir -p $(dir $@)
	$(CC) $(CFLAGS) -c $< -o $@

# RandomPool.cpp: rename GetInstance so that the harness can switch between
# the real pool and a deterministic one (shim/randpool/)
$(OBJ)/up/RandomPool.o: $(SRC)/RandomPool.cpp $(GEN_HEADERS)
	@mkdir -p $(dir $@)
	$(CXX) $(CXXFLAGS) -DGetInstance=GetInstance_real -c $< -o $@

# units whose results depend on Dinkumware log()/std::sort (shim/dinkum_compat.h)
$(OBJ)/up/PasswGen.o $(OBJ)/up/zxcvbn.o: $(OBJ)/up/%.o: $(SRC)/%.cpp $(GEN_HEADERS)
	@mkdir -p $(dir $@)
	$(CXX) $(CXXFLAGS) -include $(HERE)/shim/dinkum_compat.h -c $< -o $@

$(OBJ)/up/%.o: $(SRC)/%.cpp $(GEN_HEADERS)
	@mkdir -p $(dir $@)
	$(CXX) $(CXXFLAGS) -c $< -o $@

$(OBJ)/h/randpool_real.o: $(HERE)/shim/randpool/randpool_real.cpp $(GEN_HEADERS)
	@mkdir -p $(dir $@)
	$(CXX) $(CXXFLAGS) -DGetInstance=GetInstance_real -c $< -o $@

$(OBJ)/h/%.o: $(HERE)/%.cpp $(GEN_HEADERS)
	@mkdir -p $(dir $@)
	$(CXX) $(CXXFLAGS) -c $< -o $@
$(OBJ)/h/%.o: $(HERE)/shim/vcl/%.cpp $(GEN_HEADERS)
	@mkdir -p $(dir $@)
	$(CXX) $(CXXFLAGS) -c $< -o $@
$(OBJ)/h/%.o: $(HERE)/shim/app/%.cpp $(GEN_HEADERS)
	@mkdir -p $(dir $@)
	$(CXX) $(CXXFLAGS) -c $< -o $@
$(OBJ)/h/%.o: $(HERE)/shim/%.cpp $(GEN_HEADERS)
	@mkdir -p $(dir $@)
	$(CXX) $(CXXFLAGS) -c $< -o $@
$(OBJ)/h/%.o: $(HERE)/shim/randpool/%.cpp $(GEN_HEADERS)
	@mkdir -p $(dir $@)
	$(CXX) $(CXXFLAGS) -c $< -o $@

$(GEN) $(SRC):
	mkdir -p $@

clean:
	rm -rf $(GEN) $(SRC) $(OBJ) $(EXE)

-include $(shell find $(OBJ) -name '*.d' 2>/dev/null)

# Retired: kept only because existing CMake build trees include this exact
# path from CMakeSystem.cmake. CFLTK_TOOLCHAIN now names toolchain-v3.cmake.
# Build-input version also forces fltk-sys's existing rerun-if-env-changed.
# No compiler/system override. Wrapper injects in ALREADY configured trees where
# CMake would not evaluate a newly supplied toolchain file.
set(CMAKE_PROJECT_INCLUDE "${CMAKE_CURRENT_LIST_DIR}/inject-v1.cmake")

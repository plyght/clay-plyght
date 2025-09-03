#!/bin/bash

set -e

# Performance Benchmark Script for Clay vs Bun
echo "🏁 Clay vs Bun Performance Benchmark"
echo "====================================="

# Setup test directory
TEST_DIR="benchmark_test"
CLAY_BINARY="./target/release/clay"

if [ ! -f "$CLAY_BINARY" ]; then
    echo "❌ Clay binary not found at $CLAY_BINARY. Please run 'cargo build --release' first."
    exit 1
fi

# Check if bun is installed
if ! command -v bun &> /dev/null; then
    echo "❌ Bun is not installed. Please install Bun first."
    exit 1
fi

# Test scenarios
run_test() {
    local name="$1"
    local package_list="$2"
    local iterations=3
    
    echo ""
    echo "📦 Testing: $name"
    echo "Packages: $package_list"
    echo "----------------------------------------"
    
    # Clean test directory
    rm -rf "$TEST_DIR"
    mkdir -p "$TEST_DIR"
    cd "$TEST_DIR"
    
    # Create package.json
    cat > package.json << EOF
{
  "name": "benchmark-test",
  "version": "1.0.0",
  "dependencies": {}
}
EOF
    
    # Test Clay
    echo "🧱 Clay benchmarks:"
    clay_times=()
    for i in $(seq 1 $iterations); do
        rm -rf node_modules clay-lock.toml 2>/dev/null || true
        start_time=$(date +%s%N)
        $CLAY_BINARY install $package_list > /dev/null 2>&1
        end_time=$(date +%s%N)
        duration=$(( (end_time - start_time) / 1000000 )) # Convert to milliseconds
        clay_times+=($duration)
        echo "  Run $i: ${duration}ms"
    done
    
    # Calculate Clay average
    clay_total=0
    for time in "${clay_times[@]}"; do
        clay_total=$((clay_total + time))
    done
    clay_avg=$((clay_total / iterations))
    
    # Test Bun
    echo "🥟 Bun benchmarks:"
    bun_times=()
    for i in $(seq 1 $iterations); do
        rm -rf node_modules bun.lockb 2>/dev/null || true
        start_time=$(date +%s%N)
        bun install $package_list > /dev/null 2>&1
        end_time=$(date +%s%N)
        duration=$(( (end_time - start_time) / 1000000 )) # Convert to milliseconds
        bun_times+=($duration)
        echo "  Run $i: ${duration}ms"
    done
    
    # Calculate Bun average
    bun_total=0
    for time in "${bun_times[@]}"; do
        bun_total=$((bun_total + time))
    done
    bun_avg=$((bun_total / iterations))
    
    # Results
    echo ""
    echo "📊 Results:"
    echo "  Clay average: ${clay_avg}ms"
    echo "  Bun average:  ${bun_avg}ms"
    
    if [ $clay_avg -lt $bun_avg ]; then
        improvement=$(( ((bun_avg - clay_avg) * 100) / bun_avg ))
        echo "  🚀 Clay is ${improvement}% faster!"
    else
        deficit=$(( ((clay_avg - bun_avg) * 100) / bun_avg ))
        echo "  🐌 Clay is ${deficit}% slower"
    fi
    
    cd ..
}

# Test scenarios
run_test "Single package" "lodash"
run_test "Small project (5 packages)" "lodash moment express axios uuid"
run_test "Medium project (10+ deps)" "react react-dom @types/react @types/node typescript webpack webpack-cli babel-loader @babel/core @babel/preset-env"

# Warm cache test
echo ""
echo "🔥 Warm cache test"
echo "----------------------------------------"
cd "$TEST_DIR"
echo "Installing packages with Clay (warm)..."
start_time=$(date +%s%N)
$CLAY_BINARY install lodash > /dev/null 2>&1
end_time=$(date +%s%N)
clay_warm=$(( (end_time - start_time) / 1000000 ))

rm -rf node_modules bun.lockb
echo "Installing packages with Bun (warm)..."
start_time=$(date +%s%N)
bun install lodash > /dev/null 2>&1
end_time=$(date +%s%N)
bun_warm=$(( (end_time - start_time) / 1000000 ))

echo "Clay warm: ${clay_warm}ms"
echo "Bun warm:  ${bun_warm}ms"

cd ..

echo ""
echo "🎯 Benchmark complete!"
echo "Check above results for optimization opportunities."

# Cleanup
rm -rf "$TEST_DIR"
package main

import "testing"

func TestAdd(t *testing.T) {
	if Add(2, 3) != 5 {
		t.Fatal("2+3 should be 5")
	}
}

func TestAddZero(t *testing.T) {
	if Add(0, 0) != 0 {
		t.Fatal("0+0 should be 0")
	}
}

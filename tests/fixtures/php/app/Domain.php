<?php
namespace App;

interface Payment { public function pay(int $amount); }
class OldPayment implements Payment { public function pay(int $amount) { return 'old'; } }
class NewPayment implements Payment { public function pay(int $amount) { return 'new'; } }
trait Labels { public function label() { return 'shared'; } }
class Base {
    public function __construct() {}
    public function inherited() { return 7; }
}
class Shop extends Base {
    use Labels;
    public function __construct() { parent::__construct(); }
    public function label() { return 'shop'; }
    public function choose(Payment $payment, bool $enabled) {
        $result = $payment->pay(5);
        if (!$enabled) { return null; }
        $number = 42;
        if ($number > 0) {
            if ($enabled) { return $number; }
        }
        return $result;
    }
    public function definite() {
        $payment = new NewPayment();
        return $payment->pay(2);
    }
    public function values($kind) {
        if ($kind === 1) { return 1; }
        if ($kind === 2) { return 'two'; }
        if ($kind === 3) { return new NewPayment(); }
        if ($kind === 4) { return fopen('/never-executed', 'r'); }
        return null;
    }
}
class OtherShop extends Base { use Labels; }
function outer() {
    $callback = function () { return 'closure'; };
    function inner() { return 100; }
    return $callback();
}
function arrow() { $fn = fn(int $x) => $x + 1; return $fn(1); }
function noReturn() { $x = 1; }
function early(bool $flag) { if ($flag) { return 'yes'; } return 'no'; }

<?php
namespace yii\base;
class Component {
    public function getTitle() { return 'vendor title'; }
    public function setTitle(string $title) {}
    public function __get($name) { return null; }
    public function __call($name, $args) { return null; }
}

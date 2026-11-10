<?php
namespace App;
use yii\base\Component;
class AuditBehavior { public function audit() { return true; } }
class Mailer { public function send() { return true; } }
class Model extends Component {
    public function behaviors() { return [AuditBehavior::class]; }
    public function read() { return $this->title; }
    public function write() { $this->title = 'new'; }
    public function invoke() { return $this->audit(); }
    public function dynamic($method) { return $this->$method(); }
    public function mail() { return \Yii::$app->mailer->send(); }
}
